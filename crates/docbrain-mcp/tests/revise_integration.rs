// SPDX-License-Identifier: MIT
//! docbrain_revise sends PATCH for an edit and POST …/withdraw for a withdraw,
//! and a create refusal reaches the agent in the server's words.
use axum::routing::{patch, post};
use axum::{Json, Router};
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

async fn serve(seen: Arc<Mutex<Vec<String>>>) -> String {
    let s1 = seen.clone();
    let s2 = seen.clone();
    let app = Router::new()
        .route(
            "/api/v1/fragments/{id}",
            patch(
                move |axum::extract::Path(id): axum::extract::Path<String>, Json(b): Json<Value>| {
                    let s = s1.clone();
                    async move {
                        s.lock().expect("lock").push(format!("PATCH {id} {}", b["content"]));
                        Json(json!({"id": id, "status": "indexed"}))
                    }
                },
            ),
        )
        .route(
            "/api/v1/fragments/{id}/withdraw",
            post(
                move |axum::extract::Path(id): axum::extract::Path<String>, Json(b): Json<Value>| {
                    let s = s2.clone();
                    async move {
                        s.lock().expect("lock").push(format!("WITHDRAW {id} {}", b["reason"]));
                        Json(json!({"id": id, "status": "withdrawn", "carried_by_draft": null}))
                    }
                },
            ),
        )
        .route(
            "/api/v1/fragments",
            post(|| async {
                (
                    axum::http::StatusCode::TOO_MANY_REQUESTS,
                    Json(json!({"error": "capture limit reached: 60 an hour", "kind": "capture_limit", "retry_after_secs": 17})),
                )
            }),
        )
        .route(
            "/api/v1/auth/me",
            axum::routing::get(|| async { Json(json!({"role": "editor"})) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    format!("http://{addr}")
}

async fn call(mcp: &McpServer, tool: &str, args: Value) -> docbrain_mcp::JsonRpcResponse {
    let req: JsonRpcRequest = serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": args}
    }))
    .expect("request");
    mcp.handle_request(req).await
}

#[tokio::test]
async fn revise_edits_with_patch_and_withdraws_with_post() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let url = serve(seen.clone()).await;
    let mcp = McpServer::from_parts(url, Some("k".into()));
    mcp.validate_connection().await.expect("connect");
    let id = "7f3c0000-0000-0000-0000-000000000a91";
    let r = call(&mcp, "docbrain_revise", json!({"id": id, "text": "new text"})).await;
    assert!(r.error.is_none(), "edit: {:?}", r.error);
    let r = call(&mcp, "docbrain_revise", json!({"id": id, "withdraw_reason": "wrong port"})).await;
    assert!(r.error.is_none(), "withdraw: {:?}", r.error);
    let seen = seen.lock().expect("lock").clone();
    assert_eq!(
        seen,
        vec![format!("PATCH {id} \"new text\""), format!("WITHDRAW {id} \"wrong port\"")]
    );
}

#[tokio::test]
async fn a_rate_limited_capture_reports_the_servers_words_and_stops() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let url = serve(seen).await;
    let mcp = McpServer::from_parts(url, Some("k".into()));
    mcp.validate_connection().await.expect("connect");
    let r = call(&mcp, "docbrain_annotate", json!({"file_path": "a.rs", "annotation": "a decision"})).await;
    let text = serde_json::to_string(&r.result).expect("json");
    assert!(text.contains("capture limit reached: 60 an hour"), "{text}");
    assert!(text.contains("do not retry"), "{text}");
    assert!(text.contains("retry in 17s"), "{text}");
}
