use crate::common::{self, MockResponse};
use llagram::llama::{ChatMessage, LlamaClient};
use llagram::memory;
use llagram::prompts::Prompts;
use std::fs;

fn prompts_with(files: &[(&str, &str)]) -> (tempfile::TempDir, Prompts) {
    let dir = tempfile::tempdir().unwrap();
    for (name, body) in files {
        fs::write(dir.path().join(format!("{name}.md")), body).unwrap();
    }
    let prompts = Prompts::new(dir.path().to_path_buf());
    (dir, prompts)
}

#[tokio::test]
async fn new_memory_uses_transcript_prompt() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice(
        "user likes rust",
        "stop",
    ))]);
    let llama = LlamaClient::new(&mock.url, "").unwrap();
    let (_dir, prompts) = prompts_with(&[(
        "MEMORY_NEW",
        "Summarize:\n{transcript}",
    )]);
    let summary = memory::update_summary(
        &llama,
        &prompts,
        "",
        &[ChatMessage::user("I like rust")],
        2000,
    )
    .await
    .unwrap();
    assert_eq!(summary, "user likes rust");
    let requests = mock.requests();
    let prompt = requests[0].body["messages"][0]["content"]
        .as_str()
        .unwrap();
    assert!(prompt.contains("user: I like rust"), "{prompt}");
    assert!(prompt.contains("Summarize:"));
}

#[tokio::test]
async fn existing_memory_uses_update_prompt() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice(
        "old plus new",
        "stop",
    ))]);
    let llama = LlamaClient::new(&mock.url, "").unwrap();
    let (_dir, prompts) = prompts_with(&[(
        "MEMORY_UPDATE",
        "Existing:{existing}\nNew:{transcript}",
    )]);
    let summary = memory::update_summary(
        &llama,
        &prompts,
        "old fact",
        &[ChatMessage::assistant("later")],
        2000,
    )
    .await
    .unwrap();
    assert_eq!(summary, "old plus new");
    let requests = mock.requests();
    let prompt = requests[0].body["messages"][0]["content"]
        .as_str()
        .unwrap();
    assert!(prompt.contains("Existing:old fact"));
    assert!(prompt.contains("assistant: later"));
}

#[tokio::test]
async fn oversize_summary_is_compressed_then_truncated() {
    let long = "abcdefghij"; // 10 chars
    let still_long = "ABCDEFGHIJ";
    let mock = common::LlamaMock::spawn(vec![
        MockResponse::Json(common::chat_choice(long, "stop")),
        MockResponse::Json(common::chat_choice(still_long, "stop")),
    ]);
    let llama = LlamaClient::new(&mock.url, "").unwrap();
    let (_dir, prompts) = prompts_with(&[
        ("MEMORY_NEW", "{transcript}"),
        ("MEMORY_COMPRESS", "Shrink {summary} to {max_size}"),
    ]);
    let summary = memory::update_summary(
        &llama,
        &prompts,
        "",
        &[ChatMessage::user("x")],
        4,
    )
    .await
    .unwrap();
    assert_eq!(summary, "ABC…");
    let requests = mock.requests();
    assert_eq!(requests.len(), 2);
    let compress = requests[1].body["messages"][0]["content"]
        .as_str()
        .unwrap();
    assert!(compress.contains("Shrink abcdefghij to 4"), "{compress}");
}

#[tokio::test]
async fn missing_memory_prompts_are_an_error() {
    let mock = common::LlamaMock::spawn(vec![]);
    let llama = LlamaClient::new(&mock.url, "").unwrap();
    let (_dir, prompts) = prompts_with(&[]);
    let err = memory::update_summary(&llama, &prompts, "", &[ChatMessage::user("x")], 10)
        .await
        .unwrap_err();
    assert!(err.0.contains("MEMORY_NEW.md"));
}
