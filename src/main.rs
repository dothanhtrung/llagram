#[cfg(target_os = "linux")]
use tikv_jemallocator::Jemalloc;

#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

mod config;
mod llama;

use crate::config::Config;
use crate::llama::{
    ChatMessage,
    LlamaClient,
};
use clap::Parser;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc,
    Mutex,
};
use std::time::Duration;
use teloxide::sugar::request::RequestReplyExt;
use teloxide::types::ChatAction;
use teloxide::utils::command::BotCommands;
use teloxide::{
    Bot,
    prelude::Requester,
    requests::ResponseResult,
    types::Message,
};
use tracing_subscriber::EnvFilter;

const MAX_HISTORY: usize = 40;
const TELEGRAM_CHUNK: usize = 4000;
const TYPING_INTERVAL: Duration = Duration::from_secs(4);

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
    /// List llama.cpp models
    #[command(description = "list llama.cpp models")]
    Models,
    /// Set or show the current model
    #[command(description = "set or show the current model")]
    Model(String),
    /// Clear conversation history
    #[command(description = "clear conversation history")]
    Clear,
}

struct Session {
    model: String,
    history: Vec<ChatMessage>,
}

struct App {
    bot_username: String,
    allow_users: Vec<u64>,
    default_model: String,
    llama: LlamaClient,
    sessions: Mutex<HashMap<i64, Session>>,
}

#[tokio::main]
async fn main() {
    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
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

    let llama = LlamaClient::new(config.llama.url.clone()).expect("failed to create llama.cpp client");
    let bot = Bot::new(config.telegram.bot_token.clone());
    let me = bot.get_me().await.expect("failed to call Telegram getMe");
    let bot_username = me.username().to_string();

    if let Err(err) = bot.set_my_commands(Command::bot_commands()).await {
        tracing::warn!("failed to register Telegram commands: {err}");
    }

    tracing::info!(
        "llagram started; llama.cpp at {} ; bot @{}",
        config.llama.url,
        bot_username
    );

    let app = Arc::new(App {
        bot_username,
        allow_users: config.telegram.allow_users.clone(),
        default_model: config.llama.default_model.clone(),
        llama,
        sessions: Mutex::new(HashMap::new()),
    });

    teloxide::repl(bot, move |bot: Bot, msg: Message| {
        let app = Arc::clone(&app);
        async move { handle_message(app, bot, msg).await }
    })
    .await;
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

    let parse_text = normalize_model_command(&text, &app.bot_username);
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

async fn handle_command(app: Arc<App>, bot: Bot, msg: Message, cmd: Command) -> ResponseResult<()> {
    match cmd {
        Command::Help | Command::Start => {
            let text = format!(
                "Chat with your local llama.cpp instance.\n\n{}",
                Command::descriptions()
            );
            bot.send_message(msg.chat.id, text).await?;
        }
        Command::Models => match app.llama.list_models().await {
            Ok(models) => {
                let current = current_model(&app, msg.chat.id.0);
                let body = if models.is_empty() {
                    "No models reported by llama.cpp.".to_string()
                } else {
                    let mut lines = vec!["llama.cpp models:".to_string()];
                    for name in models {
                        if name == current {
                            lines.push(format!("- {name}  (current)"));
                        } else {
                            lines.push(format!("- {name}"));
                        }
                    }
                    lines.join("\n")
                };
                bot.send_message(msg.chat.id, body).await?;
            }
            Err(err) => {
                tracing::error!("list models failed: {err}");
                bot.send_message(msg.chat.id, format!("Failed to list models: {err}"))
                    .await?;
            }
        },
        Command::Model(name) => {
            let name = name.trim().to_string();
            if name.is_empty() {
                let current = current_model(&app, msg.chat.id.0);
                let shown = if current.is_empty() { "(llama.cpp default)".to_string() } else { current };
                bot.send_message(msg.chat.id, format!("Current model: {shown}")).await?;
            } else {
                set_model(&app, msg.chat.id.0, name.clone());
                bot.send_message(msg.chat.id, format!("Model set to {name}")).await?;
            }
        }
        Command::Clear => {
            clear_history(&app, msg.chat.id.0);
            bot.send_message(msg.chat.id, "Conversation cleared.").await?;
        }
    }
    Ok(())
}

async fn handle_chat(app: Arc<App>, bot: Bot, msg: Message, text: String) -> ResponseResult<()> {
    let chat_id = msg.chat.id.0;
    let (model, history) = {
        let mut sessions = app.sessions.lock().expect("session lock");
        let session = sessions.entry(chat_id).or_insert_with(|| Session {
            model: app.default_model.clone(),
            history: Vec::new(),
        });
        session.history.push(ChatMessage::user(text));
        trim_history(&mut session.history);
        (session.model.clone(), session.history.clone())
    };

    let model = resolve_model(&app, &model).await;
    let reply = keep_typing(bot.clone(), msg.chat.id, app.llama.chat(&model, &history)).await;

    match reply {
        Ok(answer) => {
            {
                let mut sessions = app.sessions.lock().expect("session lock");
                if let Some(session) = sessions.get_mut(&chat_id) {
                    session.history.push(ChatMessage::assistant(answer.clone()));
                    trim_history(&mut session.history);
                }
            }
            for chunk in chunk_message(&answer) {
                bot.send_message(msg.chat.id, chunk).reply_to(&msg).await?;
            }
        }
        Err(err) => {
            tracing::error!("llama.cpp chat failed: {err}");
            {
                let mut sessions = app.sessions.lock().expect("session lock");
                if let Some(session) = sessions.get_mut(&chat_id) {
                    session.history.pop();
                }
            }
            bot.send_message(msg.chat.id, format!("llama.cpp error: {err}"))
                .reply_to(&msg)
                .await?;
        }
    }
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

fn current_model(app: &App, chat_id: i64) -> String {
    let sessions = app.sessions.lock().expect("session lock");
    sessions
        .get(&chat_id)
        .map(|s| s.model.clone())
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| app.default_model.clone())
}

fn set_model(app: &App, chat_id: i64, model: String) {
    let mut sessions = app.sessions.lock().expect("session lock");
    sessions
        .entry(chat_id)
        .or_insert_with(|| Session {
            model: String::new(),
            history: Vec::new(),
        })
        .model = model;
}

fn clear_history(app: &App, chat_id: i64) {
    let mut sessions = app.sessions.lock().expect("session lock");
    if let Some(session) = sessions.get_mut(&chat_id) {
        session.history.clear();
    }
}

async fn resolve_model(app: &App, model: &str) -> String {
    if !model.is_empty() {
        return model.to_string();
    }
    if !app.default_model.is_empty() {
        return app.default_model.clone();
    }
    match app.llama.list_models().await {
        Ok(models) => models.into_iter().next().unwrap_or_else(|| "default".to_string()),
        Err(err) => {
            tracing::warn!("could not resolve model from llama.cpp: {err}");
            "default".to_string()
        }
    }
}

fn normalize_model_command<'a>(text: &'a str, bot_username: &str) -> &'a str {
    let bare = "/model";
    let mentioned = format!("/model@{bot_username}");
    if text == bare || (!bot_username.is_empty() && text.eq_ignore_ascii_case(&mentioned)) {
        "/model "
    } else {
        text
    }
}

fn trim_history(history: &mut Vec<ChatMessage>) {
    if history.len() > MAX_HISTORY {
        let drop = history.len() - MAX_HISTORY;
        history.drain(0..drop);
    }
}

fn chunk_message(text: &str) -> Vec<&str> {
    if text.len() <= TELEGRAM_CHUNK {
        return vec![text];
    }
    let mut chunks = Vec::new();
    let mut rest = text;
    while rest.len() > TELEGRAM_CHUNK {
        let mut end = TELEGRAM_CHUNK;
        while end > 0 && !rest.is_char_boundary(end) {
            end -= 1;
        }
        if let Some(idx) = rest[..end].rfind('\n') {
            if idx > 0 {
                end = idx;
            }
        }
        if end == 0 {
            break;
        }
        chunks.push(&rest[..end]);
        rest = rest[end..].trim_start_matches(['\n', '\r']);
    }
    if !rest.is_empty() {
        chunks.push(rest);
    }
    chunks
}

async fn keep_typing<T>(bot: Bot, chat_id: teloxide::types::ChatId, fut: impl std::future::Future<Output = T>) -> T {
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
