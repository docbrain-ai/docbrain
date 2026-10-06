// SPDX-License-Identifier: MIT
//! `docbrain_ask` continues a conversation (#443 D).
//!
//! The tool's schema has offered `session_id` for a long time, but `tool_ask`
//! sent only the question, so every MCP ask was a fresh conversation and a
//! follow-up was searched as if nothing came before it. The shim now forwards
//! the id, hands back the server's id for the next turn, and turns a stale or
//! foreign id into one instruction the host can act on.

use axum::{Json, Router, http::StatusCode, routing::post};
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// What the mock answers, and every body it received.
#[derive(Clone)]
struct Mock {
    reply: Arc<(StatusCode, String)>,
    seen: Arc<Mutex<Vec<Value>>>,
}

async fn ask(
    axum::extract::State(m): axum::extract::State<Mock>,
    Json(body): Json<Value>,
) -> (StatusCode, String) {
    m.seen.lock().expect("seen").push(body);
    (m.reply.0, m.reply.1.clone())
}

async fn spawn(status: StatusCode, body: &str) -> (Mock, String) {
    let m = Mock {
        reply: Arc::new((status, body.to_string())),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let app = Router::new().route("/api/v1/ask", post(ask)).with_state(m.clone());
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (m, format!("http://{addr}"))
}

async fn call_ask(base: &str, args: Value) -> Result<String, String> {
    let server = McpServer::from_parts(base.to_string(), Some("test-key".into()));
    let resp = server
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: json!({ "name": "docbrain_ask", "arguments": args }),
        })
        .await;
    match resp.error {
        Some(e) => Err(e.message),
        None => Ok(resp.result.expect("result")["content"][0]["text"]
            .as_str()
            .expect("text")
            .to_string()),
    }
}

const SID: &str = "6f1c1f9e-6d0b-4d55-9c0a-2a8b9f7f3c11";

#[tokio::test]
async fn the_session_id_is_forwarded_and_the_next_one_handed_back() {
    let answer = json!({ "answer": "It deploys with the release job.", "sources": [], "session_id": SID });
    let (mock, base) = spawn(StatusCode::OK, &answer.to_string()).await;

    let text = call_ask(&base, json!({ "question": "and on staging?", "session_id": SID }))
        .await
        .expect("ask succeeds");

    let seen = mock.seen.lock().expect("seen").clone();
    assert_eq!(seen[0]["session_id"], json!(SID), "the id must reach the server");
    assert!(
        text.ends_with(&format!(
            "<!-- session_id: {SID} — pass as session_id to docbrain_ask to ask a follow-up in this conversation -->"
        )),
        "{text}"
    );
}

#[tokio::test]
async fn a_new_conversation_sends_no_session_id() {
    let answer = json!({ "answer": "ok", "sources": [], "session_id": SID });
    let (mock, base) = spawn(StatusCode::OK, &answer.to_string()).await;
    call_ask(&base, json!({ "question": "how do we deploy?" })).await.expect("ask");
    let seen = mock.seen.lock().expect("seen").clone();
    assert!(seen[0].get("session_id").is_none(), "{}", seen[0]);
}

const START_NEW: &str = "start a new conversation: omit session_id";

#[tokio::test]
async fn a_stale_or_foreign_session_says_start_a_new_conversation() {
    for (status, body) in [
        (StatusCode::CONFLICT, "conversation expired, start a new one"),
        (StatusCode::FORBIDDEN, "Session belongs to another user"),
        (StatusCode::BAD_REQUEST, "Invalid session_id: invalid character"),
    ] {
        let (_, base) = spawn(status, body).await;
        let err = call_ask(&base, json!({ "question": "q", "session_id": SID }))
            .await
            .expect_err("must fail");
        assert_eq!(err, START_NEW, "{status}");
    }
}

#[tokio::test]
async fn every_other_error_passes_through_unchanged() {
    let (_, base) = spawn(StatusCode::BAD_REQUEST, "Question exceeds maximum length").await;
    let err = call_ask(&base, json!({ "question": "q", "session_id": SID }))
        .await
        .expect_err("must fail");
    assert!(err.contains("Question exceeds maximum length"), "{err}");
    assert_ne!(err, START_NEW);
}

#[test]
fn the_session_id_description_says_where_an_id_comes_from() {
    let server = McpServer::from_parts("http://localhost:9".into(), None);
    let resp = futures_lite_block_on(server.handle_request(JsonRpcRequest {
        jsonrpc: "2.0".into(),
        id: Some(json!(1)),
        method: "tools/list".into(),
        params: json!({}),
    }));
    let tools = resp.result.expect("result")["tools"].clone();
    let ask = tools
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["name"] == "docbrain_ask")
        .expect("docbrain_ask")
        .clone();
    let desc = ask["inputSchema"]["properties"]["session_id"]["description"]
        .as_str()
        .expect("session_id description");
    assert!(desc.contains("returned by a previous docbrain_ask"), "{desc}");
    assert!(desc.contains("Omit"), "{desc}");
}

fn futures_lite_block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(f)
}
