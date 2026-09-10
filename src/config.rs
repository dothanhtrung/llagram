use serde::Deserialize;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use web_misc::db::DBConfig;

fn default_summarize_every() -> u32 {
    8
}

fn default_summary_max_size() -> usize {
    2000
}

fn default_prompts_dir() -> PathBuf {
    PathBuf::from("prompts")
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub llama: LlamaConfig,
    #[serde(default)]
    pub chat: ChatConfig,
    #[serde(default)]
    pub db: DBConfig,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct TelegramConfig {
    #[serde(default)]
    pub bot_token: String,
    #[serde(default)]
    pub allow_users: Vec<u64>,
}

#[derive(Debug, Default, Deserialize, Clone)]
pub struct LlamaConfig {
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ChatConfig {
    /// Summarize after this many user messages in a thread.
    #[serde(default = "default_summarize_every")]
    pub summarize_every: u32,
    /// Max summary size in characters. 0 disables compression.
    #[serde(default = "default_summary_max_size")]
    pub summary_max_size: usize,
    /// Directory of markdown prompt files (GLOBAL_INSTRUCTS.md, SKILL_*.md, ...).
    #[serde(default = "default_prompts_dir")]
    pub prompts_dir: PathBuf,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            summarize_every: default_summarize_every(),
            summary_max_size: default_summary_max_size(),
            prompts_dir: default_prompts_dir(),
        }
    }
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self, Box<dyn Error>> {
        let s = fs::read_to_string(path)?;
        let mut cfg: Config = ron::from_str(&s)?;
        if cfg.db.sqlite.db_path.trim().is_empty() {
            cfg.db.sqlite.db_path = "llagram.sqlite".to_string();
        }
        if cfg.chat.prompts_dir.as_os_str().is_empty() {
            cfg.chat.prompts_dir = default_prompts_dir();
        }
        if cfg.chat.prompts_dir.is_relative() {
            if let Some(parent) = path.parent() {
                if !parent.as_os_str().is_empty() {
                    cfg.chat.prompts_dir = parent.join(&cfg.chat.prompts_dir);
                }
            }
        }
        Ok(cfg)
    }
}
