use crate::common::{self, MockResponse};
use llagram::llama::{ChatMessage, LlamaClient, ToolCall, ToolCallFn};
use serde_json::json;

fn client(url: &str, key: &str) -> LlamaClient {
    LlamaClient::new(url, key).expect("llama client")
}

#[tokio::test]
async fn chat_returns_assistant_content_and_sends_bearer_token() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice(
        "hello there",
        "stop",
    ))]);
    let llama = client(&mock.url, "secret-key");
    let reply = llama
        .chat(&[ChatMessage::user("hi")])
        .await
        .expect("chat");
    assert_eq!(reply, "hello there");

    let captured = mock.requests();
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].authorization.as_deref(),
        Some("Bearer secret-key")
    );
    assert_eq!(captured[0].body["stream"], false);
    assert_eq!(captured[0].body["cache_prompt"], true);
    assert_eq!(captured[0].body["messages"][0]["role"], "user");
    assert_eq!(captured[0].body["messages"][0]["content"], "hi");
}

#[tokio::test]
async fn empty_content_is_an_error() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice("", "stop"))]);
    let llama = client(&mock.url, "");
    let err = llama.chat(&[ChatMessage::user("hi")]).await.unwrap_err();
    assert!(err.0.contains("empty reply"));
}

#[tokio::test]
async fn http_error_is_surfaced() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Status(
        500,
        "boom".into(),
    )]);
    let llama = client(&mock.url, "");
    let err = llama
        .chat_complete(&[ChatMessage::user("hi")], None)
        .await
        .unwrap_err();
    assert!(err.0.contains("500"), "{err}");
}

#[tokio::test]
async fn splits_think_tags_out_of_the_answer() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice_full(
        "<think>plan</think>\nfinal answer",
        None,
        None,
        "stop",
    ))]);
    let llama = client(&mock.url, "");
    let turn = llama
        .chat_complete(&[ChatMessage::user("q")], None)
        .await
        .unwrap();
    assert_eq!(turn.reasoning, "plan");
    assert_eq!(turn.content, "final answer");
}

#[tokio::test]
async fn continues_when_finish_reason_is_length_and_merges_overlap() {
    let mock = common::LlamaMock::spawn(vec![
        MockResponse::Json(common::chat_choice("Hello wo", "length")),
        MockResponse::Json(common::chat_choice("Hello world!", "stop")),
    ]);
    let llama = client(&mock.url, "");
    let turn = llama
        .chat_complete(&[ChatMessage::user("q")], None)
        .await
        .unwrap();
    assert_eq!(turn.content, "Hello world!");
    assert_eq!(mock.requests().len(), 2);
}

#[tokio::test]
async fn returns_tool_calls_immediately_and_fills_missing_id() {
    let tools = json!([{
        "id": "",
        "function": { "name": "web_search", "arguments": {"query": "rust"} }
    }]);
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice_full(
        "",
        None,
        Some(tools),
        "tool_calls",
    ))]);
    let llama = client(&mock.url, "");
    let turn = llama
        .chat_complete(&[ChatMessage::user("search")], None)
        .await
        .unwrap();
    assert_eq!(turn.tool_calls.len(), 1);
    assert_eq!(turn.tool_calls[0].id, "call_0");
    assert_eq!(turn.tool_calls[0].name(), "web_search");
    assert_eq!(turn.tool_calls[0].args()["query"], "rust");
}

#[tokio::test]
async fn includes_tools_in_the_request_when_provided() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice(
        "no tools",
        "stop",
    ))]);
    let llama = client(&mock.url, "");
    let tools = [json!({"type": "function", "function": {"name": "web_search"}})];
    llama
        .chat_complete(&[ChatMessage::user("q")], Some(&tools))
        .await
        .unwrap();
    let body = &mock.requests()[0].body;
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["tools"][0]["function"]["name"], "web_search");
}

#[test]
fn tool_call_args_parse_json_string() {
    let call = ToolCall {
        id: "1".into(),
        type_: "function".into(),
        function: ToolCallFn {
            name: "web_fetch".into(),
            arguments: json!("{\"url\":\"https://example.com\"}"),
        },
    };
    assert_eq!(call.args()["url"], "https://example.com");
}

#[test]
fn chat_message_constructors_set_roles() {
    assert_eq!(ChatMessage::system("s").role, "system");
    assert_eq!(ChatMessage::user("u").content, "u");
    assert_eq!(ChatMessage::assistant("a").role, "assistant");
    let tool = ChatMessage::tool("id-1", "result");
    assert_eq!(tool.role, "tool");
    assert_eq!(tool.tool_call_id.as_deref(), Some("id-1"));
}
