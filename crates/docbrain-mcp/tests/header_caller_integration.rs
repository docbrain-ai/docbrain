//! Verifies that every outbound HTTP call from `docbrain-mcp` carries
//! `X-DocBrain-Caller: mcp-host`.
//!
//! Producer half of the surface-detection contract (the receiver lives
//! server-side — see `compute_surface`). DocBrain reads this header to set
//! `Surface::McpHost`, which defaults `tools_enabled = false`. Without
//! this header DocBrain would treat MCP-host traffic as `Surface::Web`
//! and dispatch live tools, causing double-fetches against APIs the
//! MCP host already invoked.
//!
//! The test drives `McpServer` directly via the `docbrain_mcp` lib
//! against an in-tree axum mock that captures incoming headers — no
//! subprocess gymnastics, no env-var mutation.

use axum::{
    Router,
    extract::State,
    http::HeaderMap,
    routing::{get, post},
};
use docbrain_mcp::{DOCBRAIN_CALLER_VALUE, McpServer};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// Shared header-capture state. We record `(label, caller_header_value)`
/// for each request the mock receives so tests can assert against a
/// specific endpoint.
#[derive(Clone, Default)]
struct CapturedHeaders {
    inner: Arc<Mutex<Vec<(String, String)>>>,
}

impl CapturedHeaders {
    fn push(&self, label: &str, value: String) {
        // Test code: a poisoned mutex here just means a previous test
        // panicked while holding the lock — surface it loudly.
        self.inner
            .lock()
            .expect("CapturedHeaders mutex poisoned")
            .push((label.into(), value));
    }

    fn get(&self, label: &str) -> Option<String> {
        self.inner
            .lock()
            .expect("CapturedHeaders mutex poisoned")
            .iter()
            .rev()
            .find(|(l, _)| l == label)
            .map(|(_, v)| v.clone())
    }
}

fn header_value(headers: &HeaderMap) -> String {
    headers
        .get("x-docbrain-caller")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

async fn handler_auth_me(
    State(captured): State<CapturedHeaders>,
    headers: HeaderMap,
) -> axum::Json<serde_json::Value> {
    captured.push("GET /auth/me", header_value(&headers));
    axum::Json(serde_json::json!({
        "name": "test-user",
        "email": "t@example.com",
        "role": "viewer"
    }))
}

async fn handler_ask(
    State(captured): State<CapturedHeaders>,
    headers: HeaderMap,
    _body: String,
) -> axum::Json<serde_json::Value> {
    captured.push("POST /ask", header_value(&headers));
    axum::Json(serde_json::json!({"answer": "ok", "sources": []}))
}

/// Spawns an axum mock server on a random port and returns the capture
/// handle plus the base URL (e.g. `http://127.0.0.1:54321`).
async fn spawn_mock() -> (CapturedHeaders, String) {
    let captured = CapturedHeaders::default();
    let app = Router::new()
        .route("/api/v1/auth/me", get(handler_auth_me))
        .route("/api/v1/ask", post(handler_ask))
        .with_state(captured.clone());

    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (captured, format!("http://{}", addr))
}

#[tokio::test]
async fn validate_connection_sends_mcp_host_header() {
    let (captured, base_url) = spawn_mock().await;
    // `from_parts` avoids touching process env vars (parallel-safe).
    let server = McpServer::from_parts(base_url, Some("test-key".to_string()));

    let result = server.validate_connection().await;
    assert!(
        result.is_ok(),
        "validate_connection should succeed: {:?}",
        result
    );

    assert_eq!(
        captured.get("GET /auth/me").as_deref(),
        Some(DOCBRAIN_CALLER_VALUE),
        "validate_connection must emit X-DocBrain-Caller: mcp-host"
    );
}

#[tokio::test]
async fn api_call_ask_sends_mcp_host_header() {
    let (captured, base_url) = spawn_mock().await;
    let server = McpServer::from_parts(base_url, Some("test-key".to_string()));

    let result = server
        .api_call("/api/v1/ask", &serde_json::json!({"question": "ping"}))
        .await;
    assert!(result.is_ok(), "api_call should succeed: {:?}", result);

    assert_eq!(
        captured.get("POST /ask").as_deref(),
        Some(DOCBRAIN_CALLER_VALUE),
        "api_call must emit X-DocBrain-Caller: mcp-host"
    );
}

#[tokio::test]
async fn header_is_sent_even_without_api_key() {
    // Defence-in-depth: the surface header MUST be present even on
    // anonymous calls (no DOCBRAIN_API_KEY). The downstream may 401
    // the request, but surface identification is independent of auth.
    let (captured, base_url) = spawn_mock().await;
    let server = McpServer::from_parts(base_url, None);

    // We don't care about the response (the mock returns 200 regardless);
    // we only care that the header was emitted on the wire.
    let _ = server.validate_connection().await;

    assert_eq!(
        captured.get("GET /auth/me").as_deref(),
        Some(DOCBRAIN_CALLER_VALUE),
        "anonymous calls must still emit X-DocBrain-Caller: mcp-host"
    );
}
