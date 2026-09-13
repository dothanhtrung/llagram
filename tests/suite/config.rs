use llagram::config::Config;
use std::fs;
use std::path::PathBuf;

fn write_ron(dir: &std::path::Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, body).expect("write ron");
    path
}

#[test]
fn loads_repo_sample_config() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("llagram.ron");
    let cfg = Config::from_file(&path).expect("sample llagram.ron");
    assert_eq!(cfg.llama.url, "http://127.0.0.1:8080");
    assert_eq!(cfg.server.listen_host, "127.0.0.1");
    assert_eq!(cfg.server.listen_port, 53755);
    assert_eq!(cfg.chat.summarize_every, 8);
    assert_eq!(cfg.chat.summary_max_size, 2000);
    assert_eq!(cfg.db.sqlite.db_path, "llagram.sqlite");
    assert_eq!(
        cfg.chat.prompts_dir,
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts")
    );
}

#[test]
fn empty_object_uses_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ron(dir.path(), "empty.ron", "()");
    let cfg = Config::from_file(&path).unwrap();
    assert!(cfg.telegram.bot_token.is_empty());
    assert!(cfg.telegram.allow_users.is_empty());
    assert!(cfg.llama.url.is_empty());
    assert_eq!(cfg.server.listen_host, "127.0.0.1");
    assert_eq!(cfg.server.listen_port, 53755);
    assert_eq!(cfg.chat.summarize_every, 8);
    assert_eq!(cfg.chat.summary_max_size, 2000);
    assert_eq!(cfg.db.sqlite.db_path, "llagram.sqlite");
}

#[test]
fn empty_db_path_falls_back_to_llagram_sqlite() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ron(
        dir.path(),
        "db.ron",
        r#"(db: (sqlite: (db_path: "")))"#,
    );
    let cfg = Config::from_file(&path).unwrap();
    assert_eq!(cfg.db.sqlite.db_path, "llagram.sqlite");
}

#[test]
fn relative_prompts_dir_is_resolved_against_config_parent() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ron(
        dir.path(),
        "cfg.ron",
        r#"(chat: (prompts_dir: "custom-prompts"))"#,
    );
    let cfg = Config::from_file(&path).unwrap();
    assert_eq!(cfg.chat.prompts_dir, dir.path().join("custom-prompts"));
}

#[test]
fn missing_file_is_an_error() {
    let path = PathBuf::from("/tmp/llagram-does-not-exist-test.ron");
    assert!(Config::from_file(&path).is_err());
}

#[test]
fn invalid_ron_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ron(dir.path(), "bad.ron", "this is not ron");
    assert!(Config::from_file(&path).is_err());
}

#[test]
fn allow_users_and_keys_are_parsed() {
    let dir = tempfile::tempdir().unwrap();
    let path = write_ron(
        dir.path(),
        "users.ron",
        r#"(
            telegram: (bot_token: "tok", allow_users: [1, 99]),
            llama: (url: "http://127.0.0.1:9", api_key: "llama-key"),
            server: (api_key: "http-key"),
            chat: (summarize_every: 3, summary_max_size: 40),
        )"#,
    );
    let cfg = Config::from_file(&path).unwrap();
    assert_eq!(cfg.telegram.bot_token, "tok");
    assert_eq!(cfg.telegram.allow_users, vec![1, 99]);
    assert_eq!(cfg.llama.api_key, "llama-key");
    assert_eq!(cfg.server.api_key, "http-key");
    assert_eq!(cfg.chat.summarize_every, 3);
    assert_eq!(cfg.chat.summary_max_size, 40);
}
