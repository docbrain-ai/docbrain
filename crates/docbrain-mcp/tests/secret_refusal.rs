// SPDX-License-Identifier: MIT
//! A capture refused because its text holds a secret: every capture tool relays the server's
//! words plus what to do next, and never repeats the text it sent.
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::{Value, json};

/// Invented for this test; it is never a real credential.
const VALUE: &str = "Synthetic-Pass-4417";

async fn refuse() -> (StatusCode, Json<Value>) {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({
            "error": "not saved: the content holds what looks like a password; remove it and send again",
            "kind": "secret_in_capture",
            "secret_kind": "password",
            "field": "content"
        })),
    )
}

async fn serve() -> String {
    let app = Router::new()
        .route("/api/v1/fragments", post(refuse))
        .route("/api/v1/fragments/{id}", patch(refuse))
        .route("/api/v1/fragments/{id}/withdraw", post(refuse))
        .route("/api/v1/auth/me", get(|| async { Json(json!({"role": "editor"})) }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    format!("http://{addr}")
}

async fn call(mcp: &McpServer, tool: &str, args: Value) -> String {
    let req: JsonRpcRequest = serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": tool, "arguments": args}
    }))
    .expect("request");
    let r = mcp.handle_request(req).await;
    assert!(r.error.is_none(), "{tool}: {:?}", r.error);
    let result = r.result.unwrap_or_default();
    // rev 8 §4: the refusal is a tool error, not a result a host could read as saved.
    assert_eq!(result["isError"], json!(true), "{tool}: {result}");
    result["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

#[tokio::test]
async fn every_capture_tool_relays_the_guidance_and_never_the_value() {
    let mcp = McpServer::from_parts(serve().await, Some("k".into()));
    mcp.validate_connection().await.expect("connect");
    let id = "7f3c0000-0000-0000-0000-000000000a91";
    for (tool, args) in [
        (
            "docbrain_annotate",
            json!({"file_path": "jobs/export.yaml", "annotation": format!("the export job's password is {VALUE}")}),
        ),
        (
            "docbrain_commit_capture",
            json!({
                "intent": format!("the export job's password changed to {VALUE}"),
                "diff_summary": format!("password: {VALUE}"),
                "commit_message": "export: new password"
            }),
        ),
        ("docbrain_revise", json!({"id": id, "text": format!("the export job's password is {VALUE}")})),
        ("docbrain_revise", json!({"id": id, "withdraw_reason": format!("it held the password {VALUE}")})),
    ] {
        let text = call(&mcp, tool, args).await;
        assert!(text.contains("holds what looks like a password"), "the server's words, {tool}: {text}");
        // #599 ruling 6: the agent reports and asks; it never edits or resends on its own.
        assert!(text.contains("Report this refusal to the person exactly as given"), "{tool}: {text}");
        assert!(text.contains("ask them whether that line holds a secret"), "{tool}: {text}");
        assert!(text.contains("Never edit, reword, split, encode or resend the text on your own"), "{tool}: {text}");
        assert!(!text.contains("send again;"), "{tool}: no instruction to rewrite and resend: {text}");
        assert!(!text.contains("Remove the value"), "{tool}: the old guidance is gone: {text}");
        assert!(!text.contains(VALUE), "{tool} repeated the value: {text}");
    }
}
