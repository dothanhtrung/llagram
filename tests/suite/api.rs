use crate::common::{self, MockResponse};
use actix_web::{test, web, App};
use llagram::api::{self, ApiState};
use llagram::llama::LlamaClient;
use serde_json::{Value, json};
use teloxide::Bot;

fn state(llama_url: &str, api_key: &str, allow_users: Vec<u64>) -> ApiState {
    ApiState {
        bot: Bot::new("test-token"),
        llama: LlamaClient::new(llama_url, "").expect("llama client"),
        prompts: common::prompts(),
        allow_users,
        api_key: api_key.to_string(),
    }
}

async fn call_send(state: ApiState, body: Value) -> (u16, Value) {
    let app = test::init_service(
        App::new()
            .app_data(web::Data::new(state))
            .configure(api::configure),
    )
    .await;
    let req = test::TestRequest::post()
        .uri("/send")
        .set_json(&body)
        .to_request();
    let resp = test::call_service(&app, req).await;
    let status = resp.status().as_u16();
    let json = test::read_body_json(resp).await;
    (status, json)
}

#[actix_web::test]
async fn rejects_invalid_api_key() {
    let (status, body) = call_send(
        state("http://127.0.0.1:9", "secret", vec![]),
        json!({"endpoint":"llama","msg":"hi","api_key":"wrong"}),
    )
    .await;
    assert_eq!(status, 401);
    assert_eq!(body["ok"], false);
    assert_eq!(body["error"], "invalid api_key");
}

#[actix_web::test]
async fn empty_configured_api_key_accepts_any() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice(
        "pong",
        "stop",
    ))]);
    let (status, body) = call_send(
        state(&mock.url, "", vec![]),
        json!({"endpoint":"llama","msg":"ping","api_key":"ignored"}),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["reply"], "pong");
}

#[actix_web::test]
async fn empty_message_is_bad_request() {
    let (status, body) = call_send(
        state("http://127.0.0.1:9", "k", vec![]),
        json!({"endpoint":"telegram","msg":"   ","api_key":"k"}),
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(body["error"], "msg is empty");
}

#[actix_web::test]
async fn telegram_without_allow_users_is_bad_gateway() {
    let (status, body) = call_send(
        state("http://127.0.0.1:9", "k", vec![]),
        json!({"endpoint":"telegram","msg":"hello from API","api_key":"k"}),
    )
    .await;
    assert_eq!(status, 502);
    assert!(
        body["error"]
            .as_str()
            .unwrap_or("")
            .contains("allow_users is empty"),
        "{body}"
    );
}

#[actix_web::test]
async fn llama_endpoint_returns_model_reply() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Json(common::chat_choice(
        "from llama",
        "stop",
    ))]);
    let (status, body) = call_send(
        state(&mock.url, "llagram-secret", vec![]),
        json!({
            "endpoint": "llama",
            "msg": "hello from API",
            "api_key": "llagram-secret"
        }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["reply"], "from llama");
    assert!(body.get("error").is_none());
}

#[actix_web::test]
async fn llama_backend_failure_is_bad_gateway() {
    let mock = common::LlamaMock::spawn(vec![MockResponse::Status(500, "nope".into())]);
    let (status, body) = call_send(
        state(&mock.url, "k", vec![]),
        json!({"endpoint":"llama","msg":"hi","api_key":"k"}),
    )
    .await;
    assert_eq!(status, 502);
    assert_eq!(body["ok"], false);
    assert!(body["error"].as_str().unwrap_or("").contains("500"));
}
