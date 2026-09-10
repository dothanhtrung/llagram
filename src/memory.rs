use crate::llama::{ChatMessage, LlamaClient, LlamaError};
use crate::prompts::Prompts;

pub async fn update_summary(
    llama: &LlamaClient,
    prompts: &Prompts,
    existing: &str,
    recent: &[ChatMessage],
    max_size: usize,
) -> Result<String, LlamaError> {
    let transcript = format_transcript(recent);
    let prompt = if existing.trim().is_empty() {
        prompts.render("MEMORY_NEW", &[("transcript", &transcript)])
    } else {
        prompts.render(
            "MEMORY_UPDATE",
            &[("existing", existing), ("transcript", &transcript)],
        )
    };
    if prompt.trim().is_empty() {
        return Err(LlamaError(
            "MEMORY_NEW.md / MEMORY_UPDATE.md is empty or missing".into(),
        ));
    }

    let mut summary = llama.chat(&[ChatMessage::user(prompt)]).await?;

    if max_size > 0 && summary.chars().count() > max_size {
        summary = compress_summary(llama, prompts, &summary, max_size).await?;
        if summary.chars().count() > max_size {
            summary = truncate_chars(&summary, max_size);
        }
    }
    Ok(summary)
}

async fn compress_summary(
    llama: &LlamaClient,
    prompts: &Prompts,
    summary: &str,
    max_size: usize,
) -> Result<String, LlamaError> {
    let max = max_size.to_string();
    let prompt = prompts.render(
        "MEMORY_COMPRESS",
        &[("summary", summary), ("max_size", &max)],
    );
    if prompt.trim().is_empty() {
        return Ok(truncate_chars(summary, max_size));
    }
    llama.chat(&[ChatMessage::user(prompt)]).await
}

fn format_transcript(messages: &[ChatMessage]) -> String {
    messages
        .iter()
        .map(|m| format!("{}: {}", m.role, m.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = text.chars().take(keep).collect();
    out.push('…');
    out
}
