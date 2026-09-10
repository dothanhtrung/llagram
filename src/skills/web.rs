use reqwest::Client;
use serde_json::{Value, json};
use std::net::IpAddr;
use std::time::Duration;
use tracing::warn;

const SEARCH_LIMIT: usize = 5;
const FETCH_MAX_BYTES: usize = 200_000;
const FETCH_MAX_CHARS: usize = 8_000;

#[derive(Clone)]
pub struct WebSkill {
    http: Client,
}

impl WebSkill {
    pub fn new() -> Result<Self, String> {
        let http = Client::builder()
            .timeout(Duration::from_secs(20))
            .redirect(reqwest::redirect::Policy::limited(5))
            .user_agent("llagram/0.0.1 (local telegram llama.cpp bridge)")
            .build()
            .map_err(|e| format!("web skill http client: {e}"))?;
        Ok(Self { http })
    }

    pub async fn execute(&self, tool: &str, args: Value) -> Result<String, String> {
        match tool {
            "web_search" => {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if query.is_empty() {
                    return Err("web_search requires query".into());
                }
                search(&self.http, &query).await
            }
            "web_fetch" => {
                let url = extract_url(&args).ok_or_else(|| "web_fetch requires url".to_string())?;
                fetch(&self.http, &url).await
            }
            other => Err(format!("unknown web tool '{other}'")),
        }
    }
}

fn extract_url(args: &Value) -> Option<String> {
    if let Some(url) = args.get("url").and_then(Value::as_str) {
        let url = url.trim().trim_matches(['<', '>', '"', '\'']);
        if url.starts_with("http://") || url.starts_with("https://") {
            return Some(url.to_string());
        }
        if let Some(found) = find_http_url(url) {
            return Some(found);
        }
    }
    for value in args.as_object()?.values() {
        if let Some(s) = value.as_str() {
            if let Some(found) = find_http_url(s) {
                return Some(found);
            }
        }
    }
    None
}

fn find_http_url(text: &str) -> Option<String> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let rest = &text[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || matches!(c, '>' | ')' | '"' | '\''))
        .unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches(['.', ',', ';']);
    if url.starts_with("http://") || url.starts_with("https://") {
        Some(url.to_string())
    } else {
        None
    }
}

pub fn tools() -> Vec<Value> {
    vec![
        json!({
            "type": "function",
            "function": {
                "name": "web_search",
                "description": "Search the public web for a topic. Do not use this when the user already provided a URL; use web_fetch on that URL instead.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        }
                    },
                    "required": ["query"]
                }
            }
        }),
        json!({
            "type": "function",
            "function": {
                "name": "web_fetch",
                "description": "Fetch a public http(s) page and return extracted text. Use this when the user already gave a URL. After you have the page text, answer the user; do not fetch the same URL again.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "url": {
                            "type": "string",
                            "description": "Full http or https URL"
                        }
                    },
                    "required": ["url"]
                }
            }
        }),
    ]
}

async fn search(http: &Client, query: &str) -> Result<String, String> {
    let (wiki, ddg) = tokio::join!(wikipedia_search(http, query), duckduckgo_search(http, query));
    let mut lines = Vec::new();
    let mut n = 0usize;
    if let Ok(items) = wiki {
        for (title, snippet, url) in items {
            n += 1;
            lines.push(format!("{n}. {title}\n   {url}\n   {snippet}"));
            if n >= SEARCH_LIMIT {
                break;
            }
        }
    } else if let Err(err) = wiki {
        warn!("wikipedia search failed: {err}");
    }
    if n < SEARCH_LIMIT {
        if let Ok(items) = ddg {
            for (title, snippet, url) in items {
                if lines.iter().any(|l| l.contains(&url)) {
                    continue;
                }
                n += 1;
                lines.push(format!("{n}. {title}\n   {url}\n   {snippet}"));
                if n >= SEARCH_LIMIT {
                    break;
                }
            }
        } else if let Err(err) = ddg {
            warn!("duckduckgo search failed: {err}");
        }
    }
    if lines.is_empty() {
        return Ok(format!("No web results for {query:?}."));
    }
    Ok(lines.join("\n\n"))
}

async fn wikipedia_search(
    http: &Client,
    query: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let response = http
        .get("https://en.wikipedia.org/w/api.php")
        .query(&[
            ("action", "opensearch"),
            ("format", "json"),
            ("limit", "5"),
            ("search", query),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("wikipedia HTTP {status}: {body}"));
    }
    let value: Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let titles = value.get(1).and_then(Value::as_array).cloned().unwrap_or_default();
    let descs = value.get(2).and_then(Value::as_array).cloned().unwrap_or_default();
    let urls = value.get(3).and_then(Value::as_array).cloned().unwrap_or_default();
    let mut out = Vec::new();
    for i in 0..titles.len() {
        let title = titles.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        let snippet = descs.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        let url = urls.get(i).and_then(Value::as_str).unwrap_or("").to_string();
        if !title.is_empty() {
            out.push((title, snippet, url));
        }
    }
    Ok(out)
}

async fn duckduckgo_search(
    http: &Client,
    query: &str,
) -> Result<Vec<(String, String, String)>, String> {
    let response = http
        .get("https://html.duckduckgo.com/html/")
        .query(&[("q", query)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let body = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("duckduckgo HTTP {status}"));
    }
    Ok(parse_ddg_html(&body))
}

fn parse_ddg_html(html: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(href_at) = rest.find("uddg=") {
        let after = &rest[href_at + 5..];
        let end = after.find('"').or_else(|| after.find('&')).unwrap_or(after.len());
        let encoded = &after[..end];
        let url = urlencoding_decode(encoded);
        let after_a = after.find('>').map(|i| &after[i + 1..]).unwrap_or("");
        let title_end = after_a.find("</a>").unwrap_or(after_a.len().min(120));
        let title = collapse_ws(&strip_html(&after_a[..title_end]));
        rest = &after[end.min(after.len())..];
        if url.starts_with("http") && !title.is_empty() {
            out.push((title, String::new(), url));
        }
        if out.len() >= SEARCH_LIMIT {
            break;
        }
    }
    out
}

async fn fetch(http: &Client, raw_url: &str) -> Result<String, String> {
    let url = public_http_url(raw_url)?;
    let response = http.get(url).send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("fetch HTTP {status}"));
    }
    let take = bytes.len().min(FETCH_MAX_BYTES);
    let text = String::from_utf8_lossy(&bytes[..take]);
    let extracted = collapse_ws(&strip_html(&text));
    if extracted.is_empty() {
        return Ok("(no extractable text)".into());
    }
    Ok(truncate_chars(&extracted, FETCH_MAX_CHARS))
}

fn public_http_url(raw: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(raw).map_err(|e| format!("invalid url: {e}"))?;
    if url.scheme() != "http" && url.scheme() != "https" {
        return Err("only http and https URLs are allowed".into());
    }
    let host = url.host_str().ok_or_else(|| "url has no host".to_string())?;
    if host.eq_ignore_ascii_case("localhost") || host.eq_ignore_ascii_case("::1") {
        return Err("localhost URLs are blocked".into());
    }
    if let Ok(ip) = host.parse::<IpAddr>() {
        if !ip_is_public(ip) {
            return Err("private or local IP URLs are blocked".into());
        }
    }
    Ok(url)
}

fn ip_is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast())
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || v6.is_unique_local())
        }
    }
}

fn strip_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode_entities(&out)
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn collapse_ws(text: &str) -> String {
    let mut out = String::new();
    let mut prev_space = true;
    for c in text.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(c);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn urlencoding_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = &s[i + 1..i + 3];
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        if bytes[i] == b'+' {
            out.push(b' ');
        } else {
            out.push(bytes[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
