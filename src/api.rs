use crate::config::ServerConfig;
use crate::llama::{ChatMessage, LlamaClient};
use crate::prompts::Prompts;
use actix_web::{web, HttpResponse, HttpServer};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use teloxide::prelude::Requester;
use teloxide::types::ChatId;
use teloxide::Bot;

const TG_CHUNK: usize = 4000;

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
enum EndPoint {
    Telegram,
    Llama,
}

#[derive(Clone)]
pub struct ApiState {
    pub bot: Bot,
    pub llama: LlamaClient,
    pub prompts: Prompts,
    pub allow_users: Vec<u64>,
    pub api_key: String,
}

#[derive(Deserialize)]
struct SendReq {
    endpoint: EndPoint,
    msg: String,
    api_key: String,
}

#[derive(Serialize)]
struct SendResp {
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl SendResp {
    fn ok_empty() -> Self {
        Self {
            ok: true,
            reply: None,
            error: None,
        }
    }

    fn ok_reply(reply: String) -> Self {
        Self {
            ok: true,
            reply: Some(reply),
            error: None,
        }
    }

    fn err(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            reply: None,
            error: Some(error.into()),
        }
    }
}

pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/send", web::post().to(send));
}

pub async fn run(cfg: ServerConfig, state: ApiState) -> std::io::Result<()> {
    let host = if cfg.listen_host.trim().is_empty() {
        "127.0.0.1".to_string()
    } else {
        cfg.listen_host.clone()
    };
    let port = cfg.listen_port;
    let state = Arc::new(state);
    tracing::info!("llagram HTTP API listening on {host}:{port}");
    HttpServer::new(move || {
        actix_web::App::new()
            .app_data(web::Data::from(Arc::clone(&state)))
            .configure(configure)
    })
    .bind((host.as_str(), port))?
    .workers(1)
    .run()
    .await
}

async fn send(state: web::Data<ApiState>, body: web::Json<SendReq>) -> HttpResponse {
    if !api_key_ok(&state.api_key, &body.api_key) {
        return HttpResponse::Unauthorized().json(SendResp::err("invalid api_key"));
    }
    let msg = body.msg.trim();
    if msg.is_empty() {
        return HttpResponse::BadRequest().json(SendResp::err("msg is empty"));
    }
    match body.endpoint {
        EndPoint::Telegram => match send_telegram(&state, msg).await {
            Ok(()) => HttpResponse::Ok().json(SendResp::ok_empty()),
            Err(err) => HttpResponse::BadGateway().json(SendResp::err(err)),
        },
        EndPoint::Llama => match send_llama(&state, msg).await {
            Ok(reply) => HttpResponse::Ok().json(SendResp::ok_reply(reply)),
            Err(err) => HttpResponse::BadGateway().json(SendResp::err(err)),
        },
    }
}

fn api_key_ok(expected: &str, got: &str) -> bool {
    if expected.is_empty() {
        return true;
    }
    expected == got
}

async fn send_telegram(state: &ApiState, msg: &str) -> Result<(), String> {
    if state.allow_users.is_empty() {
        return Err("telegram.allow_users is empty; nowhere to send".into());
    }
    let mut last_err = None;
    for user_id in &state.allow_users {
        let chat = ChatId(*user_id as i64);
        for chunk in chunk_message(msg) {
            if let Err(err) = state.bot.send_message(chat, chunk).await {
                last_err = Some(err.to_string());
            }
        }
    }
    match last_err {
        Some(err) => Err(err),
        None => Ok(()),
    }
}

async fn send_llama(state: &ApiState, msg: &str) -> Result<String, String> {
    let mut messages = Vec::new();
    let system = state.prompts.system_message("", &Default::default());
    if !system.is_empty() {
        messages.push(ChatMessage::system(system));
    }
    messages.push(ChatMessage::user(msg));
    state
        .llama
        .chat_complete(&messages, None)
        .await
        .map(|turn| turn.content)
        .map_err(|e| e.to_string())
}

fn chunk_message(text: &str) -> Vec<String> {
    let total = text.chars().count();
    if total <= TG_CHUNK {
        return vec![text.to_string()];
    }
    let chars: Vec<char> = text.chars().collect();
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let mut end = (i + TG_CHUNK).min(chars.len());
        if end < chars.len() {
            if let Some(rel) = chars[i..end].iter().rposition(|c| *c == '\n') {
                if rel > 0 {
                    end = i + rel;
                }
            }
        }
        if end <= i {
            end = (i + TG_CHUNK).min(chars.len());
        }
        chunks.push(chars[i..end].iter().collect());
        i = end;
        while i < chars.len() && (chars[i] == '\n' || chars[i] == '\r') {
            i += 1;
        }
    }
    chunks
}
