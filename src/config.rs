use serde::Deserialize;
use std::error::Error;
use std::fs;
use std::path::Path;

#[derive(Debug, Default, Deserialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub telegram: TelegramConfig,
    #[serde(default)]
    pub llama: LlamaConfig,
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
    #[serde(default)]
    pub default_model: String,
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self, Box<dyn Error>> {
        let s = fs::read_to_string(path)?;
        let cfg: Config = ron::from_str(&s)?;
        Ok(cfg)
    }
}
