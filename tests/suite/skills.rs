use llagram::skills::Registry;
use serde_json::json;
use std::collections::HashSet;

#[test]
fn builtin_registry_lists_web_and_is_case_insensitive() {
    let registry = Registry::builtin().expect("builtin skills");
    let names: Vec<_> = Registry::all().iter().map(|s| s.name).collect();
    assert_eq!(names, ["web"]);
    assert_eq!(Registry::canonical_name("WEB"), Some("web"));
    assert_eq!(Registry::canonical_name("web"), Some("web"));
    assert_eq!(Registry::canonical_name("nope"), None);

    assert!(registry.tools_for(&HashSet::new()).is_empty());
    let mut enabled = HashSet::new();
    enabled.insert("web".into());
    let tools = registry.tools_for(&enabled);
    let names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();
    assert_eq!(names, ["web_search", "web_fetch"]);
}

#[tokio::test]
async fn execute_validates_args_and_unknown_tools() {
    let registry = Registry::builtin().unwrap();
    let err = registry
        .execute("nope", json!({}))
        .await
        .unwrap_err();
    assert!(err.contains("unknown tool"));

    let err = registry
        .execute("web_search", json!({"query": "  "}))
        .await
        .unwrap_err();
    assert!(err.contains("query"));

    let err = registry
        .execute("web_fetch", json!({"url": "ftp://example.com"}))
        .await
        .unwrap_err();
    assert!(err.contains("url"));
}

#[tokio::test]
async fn web_fetch_blocks_localhost_and_private_ips() {
    let registry = Registry::builtin().unwrap();
    let err = registry
        .execute("web_fetch", json!({"url": "http://localhost/secret"}))
        .await
        .unwrap_err();
    assert!(err.contains("localhost"), "{err}");

    let err = registry
        .execute("web_fetch", json!({"url": "http://127.0.0.1/secret"}))
        .await
        .unwrap_err();
    assert!(err.contains("private or local IP"), "{err}");

    let err = registry
        .execute("web_fetch", json!({"url": "http://10.0.0.5/"}))
        .await
        .unwrap_err();
    assert!(err.contains("private or local IP"), "{err}");
}
