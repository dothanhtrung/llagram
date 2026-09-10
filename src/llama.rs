use serde::{Deserialize, Serialize};
use std::fmt;
use std::time::Duration;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(3600);

#[derive(Debug, Clone)]
pub struct LlamaClient {
    http: reqwest::Client,
    base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(default)]
    pub id: String,
    #[serde(rename = "type", default = "function_type")]
    pub type_: String,
    pub function: ToolCallFn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallFn {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

fn function_type() -> String {
    "function".to_string()
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: None,
        }
    }

    pub fn assistant_tools(tool_calls: Vec<ToolCall>) -> Self {
        Self {
            role: "assistant".to_string(),
            content: String::new(),
            tool_calls: Some(tool_calls),
            tool_call_id: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".to_string(),
            content: content.into(),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

impl ToolCall {
    pub fn name(&self) -> &str {
        &self.function.name
    }

    pub fn args(&self) -> serde_json::Value {
        match &self.function.arguments {
            serde_json::Value::String(s) => {
                serde_json::from_str(s).unwrap_or(serde_json::Value::Object(Default::default()))
            }
            other => other.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChatTurn {
    pub reasoning: String,
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
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
    messages: &'a [ChatMessage],
    stream: bool,
    cache_prompt: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a [serde_json::Value]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'a str>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoiceMessage {
    content: Option<String>,
    reasoning_content: Option<String>,
    reasoning: Option<String>,
    tool_calls: Option<Vec<ToolCall>>,
}

impl LlamaClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self, LlamaError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .tcp_keepalive(Duration::from_secs(15))
            .build()
            .map_err(|e| LlamaError(format!("failed to build HTTP client: {e}")))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }

    fn request<'a>(
        &'a self,
        messages: &'a [ChatMessage],
        stream: bool,
        tools: Option<&'a [serde_json::Value]>,
    ) -> ChatRequest<'a> {
        let has_tools = tools.map(|t| !t.is_empty()).unwrap_or(false);
        ChatRequest {
            messages,
            stream,
            cache_prompt: true,
            tools: if has_tools { tools } else { None },
            tool_choice: has_tools.then_some("auto"),
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/{}", self.base_url, path.trim_start_matches('/'))
    }

    pub async fn chat(&self, messages: &[ChatMessage]) -> Result<String, LlamaError> {
        let turn = self.chat_complete(messages, None).await?;
        if turn.content.trim().is_empty() {
            return Err(LlamaError("llama.cpp returned an empty reply".to_string()));
        }
        Ok(turn.content)
    }

    /// Keep requesting until llama.cpp stops (`finish_reason` != `length`).
    /// The server `-n` cap applies per request; we continue the same assistant message.
    pub async fn chat_complete(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[serde_json::Value]>,
    ) -> Result<ChatTurn, LlamaError> {
        let mut msgs = messages.to_vec();
        let mut reasoning = String::new();
        let mut finish_reason = None;
        let mut tools = tools;
        for step in 0..16 {
            let turn = self.chat_turn(&msgs, tools).await?;
            if !turn.tool_calls.is_empty() {
                return Ok(turn);
            }
            if reasoning.is_empty() && !turn.reasoning.is_empty() {
                reasoning = turn.reasoning;
            }
            finish_reason = turn.finish_reason.clone();
            match msgs.last_mut() {
                Some(last) if last.role == "assistant" && last.tool_calls.is_none() => {
                    last.content = merge_continuation(&last.content, &turn.content);
                }
                _ => msgs.push(ChatMessage::assistant(turn.content.clone())),
            }
            let so_far = msgs
                .last()
                .map(|m| m.content.chars().count())
                .unwrap_or(0);
            let truncated = finish_reason.as_deref() == Some("length");
            if !truncated || turn.content.is_empty() {
                break;
            }
            tracing::info!(
                "llama.cpp truncated (finish_reason=length), continuing step {} ({} chars so far)",
                step + 1,
                so_far
            );
            tools = None;
        }
        let content = msgs
            .last()
            .filter(|m| m.role == "assistant")
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(ChatTurn {
            reasoning,
            content,
            tool_calls: Vec::new(),
            finish_reason,
        })
    }

    pub async fn chat_turn(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[serde_json::Value]>,
    ) -> Result<ChatTurn, LlamaError> {
        let request = self.request(messages, false, tools);

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
        let Some(choice) = parsed.choices.into_iter().next() else {
            return Err(LlamaError("llama.cpp returned no choices".to_string()));
        };
        let message = choice.message;
        let reasoning = message
            .reasoning_content
            .or(message.reasoning)
            .unwrap_or_default();
        let content = message.content.unwrap_or_default();
        let (reasoning, content) = split_think_answer(&reasoning, &content);
        let mut tool_calls = message.tool_calls.unwrap_or_default();
        for (i, call) in tool_calls.iter_mut().enumerate() {
            if call.id.is_empty() {
                call.id = format!("call_{i}");
            }
            if call.type_.is_empty() {
                call.type_ = "function".to_string();
            }
        }
        tracing::info!(
            "llama.cpp reply: reasoning {} chars, answer {} chars, tools {}, finish_reason {:?}",
            reasoning.chars().count(),
            content.chars().count(),
            tool_calls.len(),
            choice.finish_reason
        );
        Ok(ChatTurn {
            reasoning,
            content,
            tool_calls,
            finish_reason: choice.finish_reason,
        })
    }
}

/// Join a truncated reply with the next generation. llama.cpp often repeats the
/// prefills instead of returning only new tokens.
fn merge_continuation(existing: &str, new: &str) -> String {
    let existing = existing.trim_end();
    let new = new.trim_start();
    if existing.is_empty() {
        return new.to_string();
    }
    if new.is_empty() {
        return existing.to_string();
    }
    if new.starts_with(existing) {
        return new.to_string();
    }
    if existing.starts_with(new) {
        return existing.to_string();
    }
    let old: Vec<char> = existing.chars().collect();
    let nxt: Vec<char> = new.chars().collect();
    let max = old.len().min(nxt.len());
    for len in (1..=max).rev() {
        if old[old.len() - len..] == nxt[..len] {
            return format!(
                "{}{}",
                existing,
                nxt[len..].iter().collect::<String>()
            );
        }
    }
    format!("{existing}{new}")
}

fn split_think_answer(reasoning: &str, content: &str) -> (String, String) {
    let mut thought = reasoning.trim().to_string();
    let mut answer = content.trim().to_string();
    if let Some(idx) = answer.find("</think>") {
        let before = answer[..idx].replace("<think>", "");
        answer = answer[idx + "</think>".len()..].trim().to_string();
        let before = before.trim().to_string();
        if thought.is_empty() {
            thought = before;
        } else if !before.is_empty() {
            thought = format!("{thought}\n{before}");
        }
    } else if thought.is_empty() && (answer.contains("<think>") || answer.starts_with("<think>")) {
        let mut splitter = ThinkSplit::new();
        let (t, a) = splitter.push(&answer);
        let (t2, a2) = splitter.finish();
        thought = format!("{t}{t2}").trim().to_string();
        answer = format!("{a}{a2}").trim().to_string();
    } else if answer.is_empty() {
        if let Some(idx) = thought.find("</think>") {
            answer = thought[idx + "</think>".len()..].trim().to_string();
            thought = thought[..idx].replace("<think>", "").trim().to_string();
        }
    }
    thought = thought.replace("<think>", "").trim().to_string();
    answer = answer.trim_start_matches("</think>").trim().to_string();
    (thought, answer)
}

enum Phase {
    Start,
    Thinking,
    Answer,
}

struct ThinkSplit {
    pending: String,
    phase: Phase,
}

impl ThinkSplit {
    fn new() -> Self {
        Self {
            pending: String::new(),
            phase: Phase::Start,
        }
    }

    fn push(&mut self, chunk: &str) -> (String, String) {
        self.pending.push_str(chunk);
        let mut thought = String::new();
        let mut answer = String::new();
        loop {
            match self.phase {
                Phase::Start => {
                    let trimmed = self.pending.trim_start();
                    if trimmed.starts_with("<think>") {
                        let ws = self.pending.len() - trimmed.len();
                        self.pending.drain(..ws + "<think>".len());
                        self.phase = Phase::Thinking;
                    } else if self.pending.chars().all(char::is_whitespace)
                        || "<think>".starts_with(trimmed)
                    {
                        break;
                    } else {
                        self.phase = Phase::Answer;
                    }
                }
                Phase::Thinking => {
                    if let Some(idx) = self.pending.find("</think>") {
                        thought.push_str(&self.pending[..idx]);
                        self.pending.drain(..idx + "</think>".len());
                        self.phase = Phase::Answer;
                    } else {
                        let keep = partial_suffix_len(&self.pending, "</think>");
                        let take = self.pending.len() - keep;
                        thought.push_str(&self.pending[..take]);
                        self.pending.drain(..take);
                        break;
                    }
                }
                Phase::Answer => {
                    answer.push_str(&self.pending);
                    self.pending.clear();
                    break;
                }
            }
        }
        (thought, answer)
    }

    fn finish(&mut self) -> (String, String) {
        if matches!(self.phase, Phase::Thinking) {
            if let Some(idx) = self.pending.find("</think>") {
                let thought = self.pending[..idx].to_string();
                let answer = self.pending[idx + "</think>".len()..].to_string();
                self.pending.clear();
                self.phase = Phase::Answer;
                return (thought, answer);
            }
            return (std::mem::take(&mut self.pending), String::new());
        }
        let (t, a) = self.push("");
        (t, format!("{a}{}", std::mem::take(&mut self.pending)))
    }
}

fn partial_suffix_len(text: &str, token: &str) -> usize {
    let max = text.len().min(token.len().saturating_sub(1));
    for n in (1..=max).rev() {
        if text.ends_with(&token[..n]) {
            return n;
        }
    }
    0
}
