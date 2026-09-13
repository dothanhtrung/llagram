use llagram::prompts::Prompts;
use serde_json::{
    Value,
    json,
};
use sqlx::SqlitePool;
use std::collections::VecDeque;
use std::io::{
    Read,
    Write,
};
use std::net::{
    TcpListener,
    TcpStream,
};
use std::path::PathBuf;
use std::sync::{
    Arc,
    Mutex,
};
use std::time::Duration;
use tempfile::TempDir;
use web_misc::db::{
    DBConfig,
    DBPool,
    PostgresConfig,
    SQLiteConfig,
};

pub struct TestDb {
    pub pool: SqlitePool,
    _dir: TempDir,
}

pub async fn test_db() -> TestDb {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("llagram-test.sqlite");
    let config = DBConfig {
        sqlite: SQLiteConfig {
            db_path: path.to_str().unwrap_or_default().to_string(),
        },
        postgres: PostgresConfig::default(),
    };
    let db = DBPool::init(&config).await.expect("failed to open sqlite database");
    sqlx::migrate!("./migrations/sqlite")
        .run(&db.sqlite_pool)
        .await
        .expect("failed to run sqlite migrations");
    let pool = db.sqlite_pool;
    TestDb { pool, _dir: dir }
}

pub fn prompts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts")
}

pub fn prompts() -> Prompts {
    Prompts::new(prompts_dir())
}

pub fn chat_choice(content: &str, finish_reason: &str) -> Value {
    json!({
        "choices": [{
            "message": { "content": content },
            "finish_reason": finish_reason
        }]
    })
}

pub fn chat_choice_full(
    content: impl Into<Value>,
    reasoning: Option<&str>,
    tool_calls: Option<Value>,
    finish_reason: &str,
) -> Value {
    let mut message = serde_json::Map::new();
    message.insert("content".into(), content.into());
    if let Some(reasoning) = reasoning {
        message.insert("reasoning_content".into(), json!(reasoning));
    }
    if let Some(tool_calls) = tool_calls {
        message.insert("tool_calls".into(), tool_calls);
    }
    json!({
        "choices": [{
            "message": Value::Object(message),
            "finish_reason": finish_reason
        }]
    })
}

#[derive(Clone, Debug)]
pub struct CapturedRequest {
    pub authorization: Option<String>,
    pub body: Value,
}

pub enum MockResponse {
    Json(Value),
    Status(u16, String),
}

pub struct LlamaMock {
    pub url: String,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    _join: std::thread::JoinHandle<()>,
}

impl LlamaMock {
    pub fn spawn(responses: Vec<MockResponse>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind llama mock");
        listener.set_nonblocking(false).expect("blocking llama mock");
        let addr = listener.local_addr().expect("local addr");
        let requests = Arc::new(Mutex::new(Vec::new()));
        let queue = Arc::new(Mutex::new(VecDeque::from(responses)));
        let requests_thread = Arc::clone(&requests);
        let join = std::thread::spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => {
                        let requests = Arc::clone(&requests_thread);
                        let queue = Arc::clone(&queue);
                        std::thread::spawn(move || {
                            handle_stream(stream, requests, queue);
                        });
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url: format!("http://{addr}"),
            requests,
            _join: join,
        }
    }

    pub fn requests(&self) -> Vec<CapturedRequest> {
        self.requests.lock().expect("requests lock").clone()
    }
}

fn handle_stream(
    mut stream: TcpStream,
    requests: Arc<Mutex<Vec<CapturedRequest>>>,
    queue: Arc<Mutex<VecDeque<MockResponse>>>,
) {
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(5))).ok();
    stream.set_nodelay(true).ok();
    loop {
        let Ok((header, body)) = read_http(&mut stream) else {
            break;
        };
        let authorization = header.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("authorization") { Some(value.trim().to_string()) } else { None }
        });
        let parsed = serde_json::from_slice(&body).unwrap_or(Value::Null);
        requests.lock().expect("requests lock").push(CapturedRequest {
            authorization,
            body: parsed,
        });
        let response = queue
            .lock()
            .expect("queue lock")
            .pop_front()
            .unwrap_or_else(|| MockResponse::Json(chat_choice("ok", "stop")));
        let (status, body) = match response {
            MockResponse::Json(value) => (200, value.to_string()),
            MockResponse::Status(status, body) => (status, body),
        };
        if write_http(&mut stream, status, &body).is_err() {
            break;
        }
    }
}

fn read_http(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut data = Vec::new();
    let mut buf = [0u8; 2048];
    loop {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "connection closed",
            ));
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(pos) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            let body_start = pos + 4;
            let header = String::from_utf8_lossy(&data[..body_start]).into_owned();
            let len = content_length(&header);
            while data.len() < body_start + len {
                let n = stream.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                data.extend_from_slice(&buf[..n]);
            }
            let end = (body_start + len).min(data.len());
            return Ok((header, data[body_start..end].to_vec()));
        }
        if data.len() > 2_000_000 {
            return Err(std::io::Error::other("request too large"));
        }
    }
}

fn content_length(header: &str) -> usize {
    header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("content-length") { value.trim().parse().ok() } else { None }
        })
        .unwrap_or(0)
}

fn write_http(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        500 => "Internal Server Error",
        _ => "Error",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
        body.len()
    );
    stream.write_all(header.as_bytes())?;
    stream.write_all(body.as_bytes())?;
    stream.flush()
}
