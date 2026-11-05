// SPDX-License-Identifier: MIT
//! `docbrain_ask` takes the Ask "Tools on/off" switch (#729).
//!
//! The console and the CLI could ask with tools off; an MCP host could not.
//! `tools_enabled: false` answers from everything DocBrain holds without
//! calling a connected tool. Only `false` is forwarded, like the console and
//! the CLI: leaving the key out keeps the server's own default and the admin
//! master switch in charge, and an explicit `true` could override a server
//! that turns tools off.

use axum::{Json, Router, http::StatusCode, routing::post};
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

async fn spawn() -> (Arc<Mutex<Vec<Value>>>, String) {
    let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = seen.clone();
    let app = Router::new().route(
        "/api/v1/ask",
        post(move |Json(body): Json<Value>| {
            let sink = sink.clone();
            async move {
                sink.lock().expect("seen").push(body);
                (StatusCode::OK, json!({ "answer": "ok", "sources": [] }).to_string())
            }
        }),
    );
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (seen, format!("http://{addr}"))
}

async fn request(server: &McpServer, method: &str, params: Value) -> Value {
    let resp = server
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params,
        })
        .await;
    assert!(resp.error.is_none(), "a tool failure is not a protocol error: {:?}", resp.error);
    resp.result.expect("result")
}

/// The body the server received for one `docbrain_ask` call, or the tool's error text.
async fn sent(args: Value) -> Result<Value, String> {
    let (seen, base) = spawn().await;
    let server = McpServer::from_parts(base, Some("test-key".into()));
    let result = request(&server, "tools/call", json!({ "name": "docbrain_ask", "arguments": args })).await;
    if result["isError"] == json!(true) {
        let text = result["content"][0]["text"].as_str().expect("text").to_string();
        assert!(seen.lock().expect("seen").is_empty(), "a refused call must not reach the server");
        return Err(text);
    }
    let seen = seen.lock().expect("seen").clone();
    assert_eq!(seen.len(), 1, "one ask, one request");
    Ok(seen[0].clone())
}

#[tokio::test]
async fn tools_off_reaches_the_server() {
    let body = sent(json!({ "question": "q", "tools_enabled": false })).await.expect("ask");
    assert_eq!(body["tools_enabled"], json!(false), "{body}");
}

#[tokio::test]
async fn tools_on_and_no_value_leave_the_choice_to_the_server() {
    for args in [
        json!({ "question": "q" }),
        json!({ "question": "q", "tools_enabled": true }),
        json!({ "question": "q", "tools_enabled": null }),
    ] {
        let body = sent(args.clone()).await.expect("ask");
        assert!(body.get("tools_enabled").is_none(), "{args} sent {body}");
    }
}

/// A value that is not a boolean is refused, never read as on or off: a host
/// that meant "off" and is silently answered with tools on would be told
/// something it did not ask for.
#[tokio::test]
async fn a_value_that_is_not_true_or_false_is_refused() {
    for bad in [json!("false"), json!(0), json!("off"), json!({}), json!([false])] {
        let err = sent(json!({ "question": "q", "tools_enabled": bad.clone() }))
            .await
            .expect_err("must be refused");
        assert!(err.contains("tools_enabled") && err.contains("true or false"), "{bad}: {err}");
    }
}

#[tokio::test]
async fn the_schema_offers_the_switch_and_says_what_off_keeps() {
    let server = McpServer::from_parts("http://localhost:9".into(), None);
    let result = request(&server, "tools/list", json!({})).await;
    let ask = result["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .find(|t| t["name"] == "docbrain_ask")
        .expect("docbrain_ask")
        .clone();
    let prop = &ask["inputSchema"]["properties"]["tools_enabled"];
    assert_eq!(prop["type"], json!("boolean"), "{ask}");
    let desc = prop["description"].as_str().expect("description");
    for word in ["docs", "captures", "claims", "connected tool"] {
        assert!(desc.contains(word), "{word} missing: {desc}");
    }
    let required = ask["inputSchema"]["required"].as_array().expect("required");
    assert_eq!(required, &vec![json!("question")], "the switch is optional");
}
