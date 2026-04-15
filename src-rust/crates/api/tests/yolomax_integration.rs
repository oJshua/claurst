//! Integration test: local mock proxy that records received headers and returns
//! canned SSE. Asserts the 5 activity tiers route correctly end-to-end.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use claurst_api::provider_types::RequestActivity;
use claurst_api::providers::YolomaxProvider;
use claurst_api::{LlmProvider, ProviderRequest};
use claurst_core::types::Message;

/// Minimal SSE response that represents a complete model response.
const CANNED_SSE: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_test\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[],\"model\":\"claude-sonnet-4-6\",\"stop_reason\":null,\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n\
event: content_block_start\n\
data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\n\
event: content_block_stop\n\
data: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\n";

/// Spawn a minimal mock proxy that records request headers and returns canned SSE.
///
/// Returns `(base_url, recorded_headers)` where `recorded_headers` is a
/// shared vec of `(method, path, headers)` tuples.
fn spawn_mock_proxy() -> (String, Arc<Mutex<Vec<HashMap<String, String>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{}", port);

    let recorded: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded_clone = recorded.clone();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(s) => s,
                Err(_) => break,
            };

            let mut buf = [0u8; 8192];
            let n = match std::io::Read::read(&mut stream, &mut buf) {
                Ok(n) => n,
                Err(_) => continue,
            };
            let request_str = String::from_utf8_lossy(&buf[..n]).to_string();

            let mut headers = HashMap::new();
            for line in request_str.lines().skip(1) {
                if line.is_empty() {
                    break;
                }
                if let Some((key, value)) = line.split_once(": ") {
                    headers.insert(key.to_lowercase(), value.to_string());
                }
            }

            recorded_clone.lock().unwrap().push(headers);

            let body = CANNED_SSE;
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: text/event-stream\r\n\
                 Connection: close\r\n\
                 \r\n\
                 {}",
                body
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    });

    (base_url, recorded)
}

fn make_request(activity: RequestActivity) -> ProviderRequest {
    ProviderRequest {
        model: "claude-sonnet-4-6".to_string(),
        messages: vec![Message::user("Hello")],
        system_prompt: None,
        tools: vec![],
        max_tokens: 100,
        temperature: None,
        top_p: None,
        top_k: None,
        stop_sequences: vec![],
        thinking: None,
        provider_options: serde_json::Value::Object(Default::default()),
        activity,
    }
}

#[tokio::test]
async fn yolomax_sends_activity_header_coding() {
    let (base_url, recorded) = spawn_mock_proxy();
    let provider = YolomaxProvider::new(
        base_url,
        "test-token".to_string(),
        "sess-1".to_string(),
        "0.1.0".to_string(),
    );

    let _ = provider.create_message(make_request(RequestActivity::Coding)).await;

    let headers = recorded.lock().unwrap();
    assert!(!headers.is_empty(), "expected at least one request");
    let h = &headers[0];
    assert_eq!(h.get("x-claurst-activity").unwrap(), "coding");
    assert_eq!(h.get("x-claurst-session-id").unwrap(), "sess-1");
    assert_eq!(h.get("x-claurst-client-version").unwrap(), "0.1.0");
    assert!(h.get("authorization").unwrap().contains("Bearer test-token"));
}

#[tokio::test]
async fn yolomax_sends_activity_header_planning() {
    let (base_url, recorded) = spawn_mock_proxy();
    let provider = YolomaxProvider::new(
        base_url,
        "tok".to_string(),
        "s".to_string(),
        "v".to_string(),
    );

    let _ = provider.create_message(make_request(RequestActivity::Planning)).await;

    let headers = recorded.lock().unwrap();
    assert_eq!(headers[0].get("x-claurst-activity").unwrap(), "planning");
}

#[tokio::test]
async fn yolomax_sends_activity_header_subagent() {
    let (base_url, recorded) = spawn_mock_proxy();
    let provider = YolomaxProvider::new(
        base_url,
        "tok".to_string(),
        "s".to_string(),
        "v".to_string(),
    );

    let _ = provider.create_message(make_request(RequestActivity::Subagent)).await;

    let headers = recorded.lock().unwrap();
    assert_eq!(headers[0].get("x-claurst-activity").unwrap(), "subagent");
}

#[tokio::test]
async fn yolomax_sends_activity_header_summarize() {
    let (base_url, recorded) = spawn_mock_proxy();
    let provider = YolomaxProvider::new(
        base_url,
        "tok".to_string(),
        "s".to_string(),
        "v".to_string(),
    );

    let _ = provider.create_message(make_request(RequestActivity::Summarize)).await;

    let headers = recorded.lock().unwrap();
    assert_eq!(headers[0].get("x-claurst-activity").unwrap(), "summarize");
}

#[tokio::test]
async fn yolomax_sends_activity_header_title() {
    let (base_url, recorded) = spawn_mock_proxy();
    let provider = YolomaxProvider::new(
        base_url,
        "tok".to_string(),
        "s".to_string(),
        "v".to_string(),
    );

    let _ = provider.create_message(make_request(RequestActivity::Title)).await;

    let headers = recorded.lock().unwrap();
    assert_eq!(headers[0].get("x-claurst-activity").unwrap(), "title");
}
