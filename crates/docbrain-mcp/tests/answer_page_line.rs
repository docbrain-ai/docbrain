// SPDX-License-Identifier: MIT
//! An answer the server gave its own page shows one visible line, "Answer page: <url>", under the
//! sources, so the reader can open it later. The id comment for docbrain_feedback stays.

use axum::{routing::post, Json, Router};
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::{json, Value};
use std::net::SocketAddr;
use tokio::net::TcpListener;

async fn spawn_mock(body: Value) -> String {
    let (ask, incident) = (body.clone(), body);
    let app = Router::new()
        .route(
            "/api/v1/ask",
            post(move || {
                let b = ask.clone();
                async move { Json(b) }
            }),
        )
        .route(
            "/api/v1/incident",
            post(move || {
                let b = incident.clone();
                async move { Json(b) }
            }),
        );
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    format!("http://{addr}")
}

async fn call(base: &str, tool: &str, args: Value) -> String {
    let server = McpServer::from_parts(base.to_string(), Some("test-key".to_string()));
    let resp = server
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: json!({ "name": tool, "arguments": args }),
        })
        .await;
    assert!(resp.error.is_none(), "{:?}", resp.error);
    resp.result.expect("result")["content"][0]["text"]
        .as_str()
        .expect("text")
        .to_string()
}

const URL: &str = "https://db.example/answers/0b7c3a52-1f00-4d6c-9e1a-3f1e2d4c5b6a";

fn body(with_url: bool) -> Value {
    let mut b = json!({
        "answer": "Reload the truststore.",
        "sources": [{ "title": "Kafka runbook", "source_url": "https://wiki.example/kafka" }],
        "episode_id": "0b7c3a52-1f00-4d6c-9e1a-3f1e2d4c5b6a",
        "answer_path": "/answers/0b7c3a52-1f00-4d6c-9e1a-3f1e2d4c5b6a"
    });
    if with_url {
        b["answer_url"] = json!(URL);
    }
    b
}

#[tokio::test]
async fn ask_shows_the_answer_page_under_the_sources() {
    let text = call(
        &spawn_mock(body(true)).await,
        "docbrain_ask",
        json!({ "question": "q" }),
    )
    .await;
    let line = format!("Answer page: {URL}");
    let at = text
        .find(&line)
        .unwrap_or_else(|| panic!("no page line: {text}"));
    assert!(
        text.find("Sources:").expect("sources") < at,
        "under the sources"
    );
    assert!(
        at < text
            .find("<!-- episode_id:")
            .expect("the feedback comment stays"),
        "above the id comment"
    );
}

#[tokio::test]
async fn incident_shows_it_too() {
    let text = call(
        &spawn_mock(body(true)).await,
        "docbrain_incident",
        json!({ "description": "brokers down" }),
    )
    .await;
    assert!(text.contains(&format!("Answer page: {URL}")), "{text}");
}

#[tokio::test]
async fn a_relative_path_alone_is_not_a_link_an_ide_can_open() {
    let text = call(
        &spawn_mock(body(false)).await,
        "docbrain_ask",
        json!({ "question": "q" }),
    )
    .await;
    assert!(!text.contains("Answer page:"), "{text}");
}
