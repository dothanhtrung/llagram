use serde::{
    Deserialize,
    Serialize,
};
use std::fmt;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone)]
pub struct LlamaClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
        }
    }
}

#[derive(Debug)]
pub struct LlamaError(pub String);

impl fmt::Display for LlamaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for LlamaError {}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [ChatMessage],
    stream: bool,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

#[derive(Deserialize)]
struct ModelInfo {
    id: String,
}

impl LlamaClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, LlamaError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| LlamaError(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub async fn list_models(&self) -> Result<Vec<String>, LlamaError> {
        let response = self
            .http
            .get(self.url("v1/models"))
            .send()
            .await
            .map_err(|e| LlamaError(format!("failed to list models: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| LlamaError(format!("failed to read models response: {e}")))?;
        if !status.is_success() {
            return Err(LlamaError(format!(
                "llama.cpp GET /v1/models returned {status}: {body}"
            )));
        }
        let parsed: ModelsResponse =
            serde_json::from_str(&body).map_err(|e| LlamaError(format!("invalid /v1/models JSON ({e}): {body}")))?;
        Ok(parsed.data.into_iter().map(|m| m.id).collect())
    }

    pub async fn chat(&self, model: &str, messages: &[ChatMessage]) -> Result<String, LlamaError> {
        let request = ChatRequest {
            model,
            messages,
            stream: false,
        };
        let response = self
            .http
            .post(self.url("v1/chat/completions"))
            .json(&request)
            .send()
            .await
            .map_err(|e| LlamaError(format!("failed to send chat request: {e}")))?;
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| LlamaError(format!("failed to read chat response: {e}")))?;
        if !status.is_success() {
            return Err(LlamaError(format!(
                "llama.cpp POST /v1/chat/completions returned {status}: {body}"
            )));
        }
        let parsed: ChatResponse = serde_json::from_str(&body)
            .map_err(|e| LlamaError(format!("invalid /v1/chat/completions JSON ({e}): {body}")))?;
        let content = parsed
            .choices
            .into_iter()
            .find_map(|choice| choice.message.content)
            .unwrap_or_default()
            .trim()
            .to_string();
        if content.is_empty() {
            return Err(LlamaError("llama.cpp returned an empty reply".to_string()));
        }
        Ok(content)
    }
}
