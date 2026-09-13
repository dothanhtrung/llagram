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
mod telegram;

use crate::config::Config;
use crate::llama::LlamaClient;
use crate::prompts::Prompts;
use crate::skills::Registry;
use crate::telegram::App;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;
use web_misc::db::DBPool;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Config file path
    #[clap(short, long, default_value = "./llagram.ron")]
    config: PathBuf,
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

    let llama = LlamaClient::new(config.llama.url.clone(), config.llama.api_key.clone())
        .expect("failed to create llama.cpp client");
    let skills = Registry::builtin().expect("failed to load skills");
    let bot = teloxide::Bot::new(config.telegram.bot_token.clone());
    let me = teloxide::prelude::Requester::get_me(&bot)
        .await
        .expect("failed to call Telegram getMe");
    let bot_username = me.username().to_string();

    let prompts = Prompts::new(config.chat.prompts_dir.clone());
    tracing::info!(
        "llagram started; llama.cpp at {} ; db {} ; prompts {} ; bot @{}",
        config.llama.url,
        config.db.sqlite.db_path,
        prompts.dir().display(),
        bot_username
    );

    let app = Arc::new(App::new(
        bot_username,
        config.telegram.allow_users.clone(),
        config.chat.summarize_every,
        config.chat.summary_max_size,
        llama,
        skills,
        prompts,
        db.sqlite_pool.clone(),
    ));

    tokio::select! {
        _ = telegram::run(bot, app) => {}
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
