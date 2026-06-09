use ollama_rs::{Ollama, generation::chat::{ChatMessage, request::ChatMessageRequest}, history};
use teloxide::{Bot, prelude::Requester, types::Message};

#[tokio::main]
async fn main() {
    // Ollama config
    let ollama = Ollama::default();
    // TODO: Send with history
    let mut history = Vec::new();
    // TODO: Store this in config
    let model = "".to_string();

    // Telegram config
    let bot = Bot::from_env();
    teloxide::repl(bot, |bot: Bot, msg: Message| async move {
        let msg_text = msg.text().unwrap_or_default().to_string();

        let user_message = ChatMessage::user(msg_text);
        let request = ChatMessageRequest::new("".to_string(), vec![user_message]);
        let response = ollama.send_chat_messages(request).await.unwrap();
        let res_msg = response.message.content;

        bot.send_message(msg.chat.id, res_msg).await?;
        Ok(())
    })
    .await;
}
