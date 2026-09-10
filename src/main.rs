#[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
use tikv_jemallocator::Jemalloc;

#[cfg(all(target_os = "linux", not(target_arch = "aarch64")))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

mod config;
mod db;
mod llama;
mod memory;
mod prompts;
mod skills;

use crate::config::Config;
use crate::llama::{ChatMessage, LlamaClient};
use crate::prompts::Prompts;
use crate::skills::Registry;
use clap::Parser;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use teloxide::sugar::request::RequestReplyExt;
use teloxide::types::{ChatAction, ChatId, MessageId};
use teloxide::utils::command::BotCommands;
use teloxide::{Bot, prelude::Requester, requests::ResponseResult, types::Message};
use tracing_subscriber::EnvFilter;
use web_misc::db::DBPool;

const MAX_HISTORY: usize = 40;
const TELEGRAM_CHUNK: usize = 4000;
const TYPING_INTERVAL: Duration = Duration::from_secs(4);
const TITLE_CHARS: usize = 60;
const MAX_TOOL_ROUNDS: usize = 4;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Config file path
    #[clap(short, long, default_value = "./llagram.ron")]
    config: PathBuf,
}

/// These commands are supported:
#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "These commands are supported:")]
enum Command {
    /// Show this help
    #[command(description = "show this help")]
    Help,
    /// Start chatting
    #[command(description = "start chatting")]
    Start,
    /// Start a new chat thread
    #[command(description = "start a new chat thread")]
    New,
    /// List chat threads
    #[command(description = "list chat threads")]
    List,
    /// Load a chat thread by id
    #[command(description = "load a chat thread by id")]
    Load(String),
    /// Delete a memory by id, or all
    #[command(description = "delete a memory by id, or all")]
    Clear(String),
    /// Enable or disable a skill
    #[command(description = "enable or disable a skill: /skill <name> on|off")]
    Skill(String),
}

struct Session {
    thread_id: Option<i64>,
    summary: String,
    history: Vec<ChatMessage>,
    chats_since_summary: u32,
}

struct App {
    bot_username: String,
    allow_users: Vec<u64>,
    summarize_every: u32,
    summary_max_size: usize,
    llama: LlamaClient,
    skills: Registry,
    prompts: Prompts,
    pool: SqlitePool,
    sessions: Mutex<HashMap<i64, Session>>,
}

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_thread_ids(true)
        .finish();
    tracing::subscriber::set_global_default(subscriber).unwrap_or_default();

    let args = Cli::parse();
    let config = Config::from_file(&args.config).expect("failed to load config");

    if config.telegram.bot_token.trim().is_empty() {
        panic!("telegram.bot_token is empty in {}", args.config.display());
    }
    if config.llama.url.trim().is_empty() {
        panic!("llama.url is empty in {}", args.config.display());
    }

    let db = DBPool::init(&config.db)
        .await
        .expect("failed to open sqlite database");
    sqlx::migrate!("./migrations/sqlite")
        .run(&db.sqlite_pool)
        .await
        .expect("failed to run sqlite migrations");

    let llama = LlamaClient::new(config.llama.url.clone()).expect("failed to create llama.cpp client");
    let skills = Registry::builtin().expect("failed to load skills");
    let bot = Bot::new(config.telegram.bot_token.clone());
    let me = bot.get_me().await.expect("failed to call Telegram getMe");
    let bot_username = me.username().to_string();

    if let Err(err) = bot.set_my_commands(Command::bot_commands()).await {
        tracing::warn!("failed to register Telegram commands: {err}");
    }

    let prompts = Prompts::new(config.chat.prompts_dir.clone());
    tracing::info!(
        "llagram started; llama.cpp at {} ; db {} ; prompts {} ; bot @{}",
        config.llama.url,
        config.db.sqlite.db_path,
        prompts.dir().display(),
        bot_username
    );

    let app = Arc::new(App {
        bot_username,
        allow_users: config.telegram.allow_users.clone(),
        summarize_every: config.chat.summarize_every,
        summary_max_size: config.chat.summary_max_size,
        llama,
        skills,
        prompts,
        pool: db.sqlite_pool.clone(),
        sessions: Mutex::new(HashMap::new()),
    });

    tokio::select! {
        _ = teloxide::repl(bot, move |bot: Bot, msg: Message| {
            let app = Arc::clone(&app);
            async move { handle_message(app, bot, msg).await }
        }) => {}
        code = shutdown_signal() => {
            tracing::info!("signal received, exiting");
            std::process::exit(code);
        }
    }
}

async fn shutdown_signal() -> i32 {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("failed to listen for SIGINT");
        let mut sigterm = signal(SignalKind::terminate()).expect("failed to listen for SIGTERM");
        tokio::select! {
            _ = sigint.recv() => 130,
            _ = sigterm.recv() => 143,
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to listen for Ctrl-C");
        130
    }
}

async fn handle_message(app: Arc<App>, bot: Bot, msg: Message) -> ResponseResult<()> {
    if !app.is_allowed(&msg) {
        bot.send_message(msg.chat.id, "You are not allowed to use this bot")
            .await?;
        return Ok(());
    }

    let Some(text) = msg.text().map(str::to_string) else {
        return Ok(());
    };

    let parse_text = normalize_arg_command(&text, &app.bot_username);
    if let Ok(cmd) = Command::parse(parse_text, &app.bot_username) {
        return handle_command(app, bot, msg, cmd).await;
    }

    if text.starts_with('/') {
        bot.send_message(msg.chat.id, "Unknown command. Try /help.")
            .reply_to(&msg)
            .await?;
        return Ok(());
    }

    handle_chat(app, bot, msg, text).await
}

async fn handle_command(
    app: Arc<App>,
    bot: Bot,
    msg: Message,
    cmd: Command,
) -> ResponseResult<()> {
    match cmd {
        Command::Help | Command::Start => {
            let text = format!(
                "Chat with your local llama.cpp instance.\n\n{}",
                Command::descriptions()
            );
            bot.send_message(msg.chat.id, text).await?;
        }
        Command::New => match start_new_thread(&app, &msg).await {
            Ok(thread) => {
                bot.send_message(msg.chat.id, format!("Started thread {}.", thread.id))
                    .await?;
            }
            Err(err) => {
                tracing::error!("new thread failed: {err}");
                bot.send_message(msg.chat.id, format!("Failed to start thread: {err}"))
                    .await?;
            }
        },
        Command::List => match list_threads_text(&app, &msg).await {
            Ok(body) => {
                bot.send_message(msg.chat.id, body).await?;
            }
            Err(err) => {
                tracing::error!("list threads failed: {err}");
                bot.send_message(msg.chat.id, format!("Failed to list threads: {err}"))
                    .await?;
            }
        },
        Command::Load(id) => {
            let id = id.trim();
            if id.is_empty() {
                bot.send_message(msg.chat.id, "Usage: /load <id>").await?;
            } else {
                match id.parse::<i64>() {
                    Ok(thread_id) => match load_thread(&app, &msg, thread_id).await {
                        Ok(thread) => {
                            let preview = preview_text(&thread.summary, 400);
                            let body = if preview.is_empty() {
                                format!("Loaded thread {} ({}). Memory is empty.", thread.id, thread.display_title())
                            } else {
                                format!(
                                    "Loaded thread {} ({}).\n\nMemory:\n{preview}",
                                    thread.id,
                                    thread.display_title()
                                )
                            };
                            bot.send_message(msg.chat.id, body).await?;
                        }
                        Err(err) => {
                            bot.send_message(msg.chat.id, err).await?;
                        }
                    },
                    Err(_) => {
                        bot.send_message(msg.chat.id, "Usage: /load <id>").await?;
                    }
                }
            }
        }
        Command::Clear(arg) => {
            let arg = arg.trim();
            if arg.is_empty() {
                bot.send_message(msg.chat.id, "Usage: /clear <id> or /clear all")
                    .await?;
            } else {
                match clear_memory(&app, &msg, arg).await {
                    Ok(text) => {
                        bot.send_message(msg.chat.id, text).await?;
                    }
                    Err(err) => {
                        bot.send_message(msg.chat.id, err).await?;
                    }
                }
            }
        }
        Command::Skill(arg) => match set_skill_command(&app, &msg, arg.trim()).await {
            Ok(text) => {
                bot.send_message(msg.chat.id, text).await?;
            }
            Err(err) => {
                bot.send_message(msg.chat.id, err).await?;
            }
        },
    }
    Ok(())
}

async fn handle_chat(
    app: Arc<App>,
    bot: Bot,
    msg: Message,
    text: String,
) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    let Some(user_id) = telegram_user_id(&msg) else {
        bot.send_message(msg.chat.id, "Cannot chat without a Telegram user id.")
            .await?;
        return Ok(());
    };

    let (thread_id, summary, history) = match prepare_turn(&app, chat_id, user_id, &text).await {
        Ok(turn) => turn,
        Err(err) => {
            tracing::error!("prepare turn failed: {err}");
            bot.send_message(msg.chat.id, format!("Failed to open thread: {err}"))
                .await?;
            return Ok(());
        }
    };

    let enabled = db::enabled_skills(&app.pool, user_id).await.unwrap_or_default();
    let tools = app.skills.tools_for(&enabled);
    let messages = build_messages(&app.prompts, &summary, &history, &enabled);
    let _ = bot.send_chat_action(msg.chat.id, ChatAction::Typing).await;
    let thinking_msg = bot
        .send_message(msg.chat.id, "Thinking…")
        .reply_to(&msg)
        .await?;

    let reply = if tools.is_empty() {
        keep_typing(
            bot.clone(),
            msg.chat.id,
            app.llama.chat_complete(&messages, None),
        )
        .await
        .map(|turn| (turn.reasoning, turn.content, turn.tool_calls))
    } else {
        tool_reply(
            &app,
            &bot,
            msg.chat.id,
            thinking_msg.id,
            messages,
            tools,
        )
        .await
        .map(|(reasoning, content)| (reasoning, content, Vec::new()))
    };

    match reply {
        Ok((_reasoning, content, tool_calls)) => {
            if !tool_calls.is_empty() {
                tracing::warn!("llama.cpp returned tool calls with skills disabled");
            }
            let answer = content.trim().to_string();
            if answer.is_empty() {
                let _ = bot.delete_message(msg.chat.id, thinking_msg.id).await;
                bot.send_message(msg.chat.id, "llama.cpp returned an empty reply.")
                    .reply_to(&msg)
                    .await?;
                pop_last_user(&app, chat_id);
                return Ok(());
            }
            for chunk in chunk_message(&answer) {
                bot.send_message(msg.chat.id, chunk).reply_to(&msg).await?;
            }
            let _ = bot.delete_message(msg.chat.id, thinking_msg.id).await;
            let should_summarize = {
                let mut sessions = app.sessions.lock().expect("session lock");
                if let Some(session) = sessions.get_mut(&chat_id) {
                    session.history.push(ChatMessage::assistant(answer));
                    trim_history(&mut session.history);
                    session.chats_since_summary += 1;
                    app.summarize_every > 0 && session.chats_since_summary >= app.summarize_every
                } else {
                    false
                }
            };
            if should_summarize {
                if let Err(err) = summarize_thread(&app, chat_id, thread_id).await {
                    tracing::error!("thread {thread_id} summary failed: {err}");
                }
            }
        }
        Err(err) => {
            tracing::error!("llama.cpp chat failed: {err}");
            pop_last_user(&app, chat_id);
            let _ = bot.delete_message(msg.chat.id, thinking_msg.id).await;
            bot.send_message(msg.chat.id, format!("llama.cpp error: {err}"))
                .reply_to(&msg)
                .await?;
        }
    }
    Ok(())
}

async fn keep_typing<T>(
    bot: Bot,
    chat_id: ChatId,
    fut: impl std::future::Future<Output = T>,
) -> T {
    tokio::select! {
        result = fut => result,
        _ = async {
            loop {
                let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
                tokio::time::sleep(TYPING_INTERVAL).await;
            }
        } => unreachable!(),
    }
}

async fn tool_reply(
    app: &App,
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    mut messages: Vec<ChatMessage>,
    tools: Vec<serde_json::Value>,
) -> Result<(String, String), crate::llama::LlamaError> {
    let mut last_reasoning = String::new();
    let mut used_tools = false;
    for round in 0..MAX_TOOL_ROUNDS {
        let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
        let turn = app.llama.chat_turn(&messages, Some(&tools)).await?;
        if !turn.reasoning.is_empty() {
            last_reasoning = turn.reasoning.clone();
            let _ = bot
                .edit_message_text(chat_id, message_id, format_thought(&last_reasoning, false))
                .await;
        }
        if turn.tool_calls.is_empty() {
            if turn.content.trim().is_empty() {
                break;
            }
            if turn.finish_reason.as_deref() == Some("length") {
                let mut cont = messages.clone();
                cont.push(ChatMessage::assistant(turn.content.clone()));
                let continued = app.llama.chat_complete(&cont, None).await?;
                if !continued.reasoning.is_empty() {
                    last_reasoning = continued.reasoning;
                }
                return Ok((last_reasoning, continued.content));
            }
            return Ok((last_reasoning, turn.content));
        }
        used_tools = true;
        messages.push(ChatMessage::assistant_tools(turn.tool_calls.clone()));
        let mut fetched_page = false;
        for call in turn.tool_calls {
            let label = format!("Using {}…", call.name());
            let _ = bot.edit_message_text(chat_id, message_id, label).await;
            tracing::info!("skill tool {} round {round}: {}", call.name(), call.args());
            let result = match app.skills.execute(call.name(), call.args()).await {
                Ok(text) => text,
                Err(err) => format!("tool error: {err}"),
            };
            if call.name() == "web_fetch" && !result.starts_with("tool error:") {
                fetched_page = true;
            }
            messages.push(ChatMessage::tool(&call.id, result));
        }
        if fetched_page {
            break;
        }
    }
    if used_tools {
        let _ = bot
            .edit_message_text(chat_id, message_id, "Writing the answer…")
            .await;
        let after_tools = app.prompts.read("AFTER_TOOLS");
        if !after_tools.is_empty() {
            messages.push(ChatMessage::user(after_tools));
        }
        let turn = app.llama.chat_complete(&messages, None).await?;
        if !turn.reasoning.is_empty() {
            last_reasoning = turn.reasoning;
        }
        if !turn.content.trim().is_empty() {
            return Ok((last_reasoning, turn.content));
        }
    }
    Err(crate::llama::LlamaError(
        "llama.cpp did not produce an answer after using tools".to_string(),
    ))
}

fn format_thought(thought: &str, done: bool) -> String {
    let header = if done { "Thought:\n\n" } else { "Thinking…\n\n" };
    let body = thought.trim();
    if body.is_empty() {
        return header.trim().to_string();
    }
    let max = TELEGRAM_CHUNK.saturating_sub(header.len());
    format!("{header}{}", tail_chars(body, max))
}

fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let skip = count - max.saturating_sub(1);
    let mut out = String::from("…");
    out.extend(text.chars().skip(skip));
    out
}

fn pop_last_user(app: &App, chat_id: i64) {
    let mut sessions = app.sessions.lock().expect("session lock");
    if let Some(session) = sessions.get_mut(&chat_id) {
        session.history.pop();
    }
}

async fn prepare_turn(
    app: &App,
    chat_id: i64,
    user_id: i64,
    text: &str,
) -> Result<(i64, String, Vec<ChatMessage>), String> {
    let mut thread_id = {
        let mut sessions = app.sessions.lock().expect("session lock");
        session_mut(&mut sessions, chat_id).thread_id
    };

    if thread_id.is_none() {
        if let Some(saved_id) = db::get_current_thread_id(&app.pool, user_id)
            .await
            .map_err(|e| e.to_string())?
        {
            if let Ok(thread) = db::get_thread(&app.pool, saved_id).await {
                if thread.user_id == user_id {
                    apply_thread(app, chat_id, &thread);
                    thread_id = Some(thread.id);
                }
            }
        }
    }

    if thread_id.is_none() {
        let thread = db::create_thread(&app.pool, user_id)
            .await
            .map_err(|e| e.to_string())?;
        apply_thread(app, chat_id, &thread);
        thread_id = Some(thread.id);
    }

    let thread_id = thread_id.expect("thread id");
    let title = make_title(text);
    if let Err(err) = db::set_title_if_empty(&app.pool, thread_id, &title).await {
        tracing::warn!("failed to set thread title: {err}");
    }

    let (summary, history) = {
        let mut sessions = app.sessions.lock().expect("session lock");
        let session = session_mut(&mut sessions, chat_id);
        session.history.push(ChatMessage::user(text));
        trim_history(&mut session.history);
        (session.summary.clone(), session.history.clone())
    };

    Ok((thread_id, summary, history))
}

async fn start_new_thread(app: &App, msg: &Message) -> Result<db::Thread, String> {
    let user_id = telegram_user_id(msg).ok_or_else(|| "missing Telegram user id".to_string())?;
    let thread = db::create_thread(&app.pool, user_id)
        .await
        .map_err(|e| e.to_string())?;
    apply_thread(app, msg.chat.id.0, &thread);
    Ok(thread)
}

async fn list_threads_text(app: &App, msg: &Message) -> Result<String, String> {
    let user_id = telegram_user_id(msg).ok_or_else(|| "missing Telegram user id".to_string())?;
    let threads = db::list_threads(&app.pool, user_id)
        .await
        .map_err(|e| e.to_string())?;
    if threads.is_empty() {
        return Ok("No chat threads yet. Send a message or /new.".to_string());
    }
    let current = current_thread_id(app, msg.chat.id.0, user_id).await;
    let mut lines = vec!["Chat threads:".to_string()];
    for thread in threads {
        let mark = if current == Some(thread.id) { "*" } else { " " };
        lines.push(format!(
            "{mark} [{}] {}  ({})",
            thread.id,
            thread.display_title(),
            thread.updated_at
        ));
    }
    Ok(lines.join("\n"))
}

async fn load_thread(app: &App, msg: &Message, thread_id: i64) -> Result<db::Thread, String> {
    let user_id = telegram_user_id(msg).ok_or_else(|| "missing Telegram user id".to_string())?;
    let thread = db::get_thread(&app.pool, thread_id)
        .await
        .map_err(|_| format!("Thread {thread_id} not found."))?;
    if thread.user_id != user_id {
        return Err(format!("Thread {thread_id} not found."));
    }
    apply_thread(app, msg.chat.id.0, &thread);
    db::set_current_thread(&app.pool, user_id, Some(thread.id))
        .await
        .map_err(|e| e.to_string())?;
    Ok(thread)
}

async fn clear_memory(app: &App, msg: &Message, arg: &str) -> Result<String, String> {
    let user_id = telegram_user_id(msg).ok_or_else(|| "missing Telegram user id".to_string())?;
    let chat_id = msg.chat.id.0;
    if arg.eq_ignore_ascii_case("all") {
        let n = db::delete_all_threads(&app.pool, user_id)
            .await
            .map_err(|e| e.to_string())?;
        reset_session(app, chat_id);
        return Ok(format!("Deleted {n} memories."));
    }
    let thread_id = arg
        .parse::<i64>()
        .map_err(|_| "Usage: /clear <id> or /clear all".to_string())?;
    let deleted = db::delete_thread(&app.pool, user_id, thread_id)
        .await
        .map_err(|e| e.to_string())?;
    if !deleted {
        return Err(format!("Memory {thread_id} not found."));
    }
    let current = {
        let sessions = app.sessions.lock().expect("session lock");
        sessions.get(&chat_id).and_then(|s| s.thread_id)
    };
    if current == Some(thread_id) {
        reset_session(app, chat_id);
    }
    Ok(format!("Deleted memory {thread_id}."))
}

async fn summarize_thread(app: &App, chat_id: i64, thread_id: i64) -> Result<(), String> {
    let (summary, history) = {
        let sessions = app.sessions.lock().expect("session lock");
        let Some(session) = sessions.get(&chat_id) else {
            return Ok(());
        };
        (session.summary.clone(), session.history.clone())
    };
    if history.is_empty() {
        return Ok(());
    }

    let updated = memory::update_summary(
        &app.llama,
        &app.prompts,
        &summary,
        &history,
        app.summary_max_size,
    )
        .await
        .map_err(|e| e.to_string())?;

    db::update_summary(&app.pool, thread_id, &updated)
        .await
        .map_err(|e| e.to_string())?;

    {
        let mut sessions = app.sessions.lock().expect("session lock");
        if let Some(session) = sessions.get_mut(&chat_id) {
            if session.thread_id == Some(thread_id) {
                session.summary = updated;
                session.history.clear();
                session.chats_since_summary = 0;
            }
        }
    }
    tracing::info!("updated summary for thread {thread_id}");
    Ok(())
}

impl App {
    fn is_allowed(&self, msg: &Message) -> bool {
        if self.allow_users.is_empty() {
            return true;
        }
        msg.from
            .as_ref()
            .map(|user| self.allow_users.contains(&user.id.0))
            .unwrap_or(false)
    }
}

fn session_mut(sessions: &mut HashMap<i64, Session>, chat_id: i64) -> &mut Session {
    sessions.entry(chat_id).or_insert_with(|| Session {
        thread_id: None,
        summary: String::new(),
        history: Vec::new(),
        chats_since_summary: 0,
    })
}

fn apply_thread(app: &App, chat_id: i64, thread: &db::Thread) {
    let mut sessions = app.sessions.lock().expect("session lock");
    let session = session_mut(&mut sessions, chat_id);
    session.thread_id = Some(thread.id);
    session.summary = thread.summary.clone();
    session.history.clear();
    session.chats_since_summary = 0;
}

fn reset_session(app: &App, chat_id: i64) {
    let mut sessions = app.sessions.lock().expect("session lock");
    if let Some(session) = sessions.get_mut(&chat_id) {
        session.thread_id = None;
        session.summary.clear();
        session.history.clear();
        session.chats_since_summary = 0;
    }
}

async fn current_thread_id(app: &App, chat_id: i64, user_id: i64) -> Option<i64> {
    {
        let sessions = app.sessions.lock().expect("session lock");
        if let Some(id) = sessions.get(&chat_id).and_then(|s| s.thread_id) {
            return Some(id);
        }
    }
    db::get_current_thread_id(&app.pool, user_id).await.ok().flatten()
}

fn build_messages(
    prompts: &Prompts,
    summary: &str,
    history: &[ChatMessage],
    enabled_skills: &std::collections::HashSet<String>,
) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len() + 1);
    let system = prompts.system_message(summary, enabled_skills);
    if !system.is_empty() {
        messages.push(ChatMessage::system(system));
    }
    messages.extend(history.iter().cloned());
    messages
}

fn normalize_arg_command<'a>(text: &'a str, bot_username: &str) -> &'a str {
    for name in ["load", "clear", "skill"] {
        let bare = format!("/{name}");
        let mentioned = format!("/{name}@{bot_username}");
        if text == bare || (!bot_username.is_empty() && text.eq_ignore_ascii_case(&mentioned)) {
            return match name {
                "load" => "/load ",
                "clear" => "/clear ",
                "skill" => "/skill ",
                _ => text,
            };
        }
    }
    text
}

async fn set_skill_command(app: &App, msg: &Message, arg: &str) -> Result<String, String> {
    let user_id = telegram_user_id(msg).ok_or_else(|| "missing Telegram user id".to_string())?;
    let mut parts = arg.split_whitespace();
    let name = parts.next();
    let state = parts.next();
    let enabled = db::enabled_skills(&app.pool, user_id)
        .await
        .map_err(|e| e.to_string())?;
    match (name, state) {
        (None, _) | (Some(""), _) => Ok(skill_list_text(&enabled)),
        (Some(name), None) => {
            let canonical = Registry::canonical_name(name)
                .ok_or_else(|| format!("Unknown skill '{name}'. {}", skill_names_hint()))?;
            let on = enabled.contains(canonical);
            Ok(format!(
                "Skill {canonical} is {}.",
                if on { "on" } else { "off" }
            ))
        }
        (Some(name), Some(flag)) => {
            let canonical = Registry::canonical_name(name)
                .ok_or_else(|| format!("Unknown skill '{name}'. {}", skill_names_hint()))?;
            let on = parse_on_off(flag)?;
            db::set_skill(&app.pool, user_id, canonical, on)
                .await
                .map_err(|e| e.to_string())?;
            Ok(format!(
                "Skill {canonical} turned {}.",
                if on { "on" } else { "off" }
            ))
        }
    }
}

fn parse_on_off(flag: &str) -> Result<bool, String> {
    match flag.to_ascii_lowercase().as_str() {
        "on" | "true" | "1" | "enable" | "enabled" => Ok(true),
        "off" | "false" | "0" | "disable" | "disabled" => Ok(false),
        _ => Err("Usage: /skill <name> on|off".to_string()),
    }
}

fn skill_list_text(enabled: &std::collections::HashSet<String>) -> String {
    let mut lines = vec!["Skills:".to_string()];
    for info in Registry::all() {
        let state = if enabled.contains(info.name) { "on" } else { "off" };
        lines.push(format!("- {}  {state}  {}", info.name, info.description));
    }
    lines.push("Use /skill <name> on|off".to_string());
    lines.join("\n")
}

fn skill_names_hint() -> String {
    let names: Vec<&str> = Registry::all().iter().map(|s| s.name).collect();
    format!("Available: {}", names.join(", "))
}

fn telegram_user_id(msg: &Message) -> Option<i64> {
    msg.from.as_ref().map(|user| user.id.0 as i64)
}

fn make_title(text: &str) -> String {
    let trimmed = text.trim();
    let mut title: String = trimmed.chars().take(TITLE_CHARS).collect();
    if trimmed.chars().count() > TITLE_CHARS {
        title.push('…');
    }
    title
}

fn preview_text(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn trim_history(history: &mut Vec<ChatMessage>) {
    if history.len() > MAX_HISTORY {
        let drop = history.len() - MAX_HISTORY;
        history.drain(0..drop);
    }
}

fn chunk_message(text: &str) -> Vec<String> {
    let total = text.chars().count();
    if total <= TELEGRAM_CHUNK {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let mut end = (i + TELEGRAM_CHUNK).min(chars.len());
        if end < chars.len() {
            if let Some(rel) = chars[i..end].iter().rposition(|c| *c == '\n') {
                if rel > 0 {
                    end = i + rel;
                }
            }
        }
        if end <= i {
            end = (i + TELEGRAM_CHUNK).min(chars.len());
        }
        chunks.push(chars[i..end].iter().collect());
        i = end;
        while i < chars.len() && (chars[i] == '\n' || chars[i] == '\r') {
            i += 1;
        }
    }
    chunks
}


