// SPDX-License-Identifier: MIT
//! The tools reach DocBrain only through a `Backend` (#601): the stdio binary's network backend
//! and the server's in-process one run the same tool code, so both doors send the same requests.
//! These tests drive `McpServer` with a recording backend and pin what each tool sends.

use docbrain_mcp::backend::{ApiRequest, ApiResponse, Backend, Method};
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

type Responder = dyn Fn(&ApiRequest) -> Result<ApiResponse, String> + Send + Sync;

#[derive(Clone)]
struct Recording {
    sent: Arc<Mutex<Vec<ApiRequest>>>,
    respond: Arc<Responder>,
}

impl Recording {
    fn answering(respond: impl Fn(&ApiRequest) -> Result<ApiResponse, String> + Send + Sync + 'static) -> Self {
        Self { sent: Arc::default(), respond: Arc::new(respond) }
    }
    fn ok(body: Value) -> Self {
        let body = body.to_string();
        Self::answering(move |_| Ok(ApiResponse { status: 200, body: body.clone() }))
    }
    fn sent(&self) -> Vec<ApiRequest> {
        self.sent.lock().expect("recording lock").clone()
    }
}

impl Backend for Recording {
    async fn send(&self, request: ApiRequest) -> Result<ApiResponse, String> {
        let answer = (self.respond)(&request);
        self.sent.lock().expect("recording lock").push(request);
        answer
    }
}

async fn call(server: &McpServer<Recording>, tool: &str, arguments: Value) -> (Option<Value>, Option<i32>) {
    let request: JsonRpcRequest = serde_json::from_value(json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": tool, "arguments": arguments }
    }))
    .expect("request");
    let response = server.handle_request(request).await;
    (response.result, response.error.map(|e| e.code))
}

fn text(result: &Value) -> String {
    result["content"][0]["text"].as_str().unwrap_or_default().to_string()
}

const ID: &str = "0b9c3f5e-6a1d-4d8e-9f2a-3c4b5d6e7f80";

#[tokio::test]
async fn every_tool_sends_its_request_through_the_backend() {
    // One canned body that every tool can read without failing.
    let backend = Recording::ok(json!({
        "answer": "a", "sources": [], "summary": {}, "documents": [], "checked_paths": ["a.rs"], "blocks": [],
        "id": ID, "draft_id": "", "title": "t", "total_gaps": 0
    }));
    let server = McpServer::with_backend(backend.clone());
    let cases: Vec<(&str, Value, Method, &str)> = vec![
        ("docbrain_ask", json!({"question": "q"}), Method::POST, "/api/v1/ask"),
        ("docbrain_incident", json!({"description": "d"}), Method::POST, "/api/v1/incident"),
        ("docbrain_freshness", json!({"space": "ops docs"}), Method::GET, "/api/v1/freshness"),
        ("docbrain_autopilot_summary", json!({}), Method::GET, "/api/v1/autopilot/summary"),
        ("docbrain_annotate", json!({"file_path": "a.rs", "annotation": "why"}), Method::POST, "/api/v1/fragments"),
        ("docbrain_context", json!({"file_paths": ["a.rs"]}), Method::POST, "/api/v1/fragments/context"),
        ("docbrain_commit_capture", json!({"intent": "why"}), Method::POST, "/api/v1/fragments"),
        ("docbrain_feedback", json!({"episode_id": "e", "thumbs_up": true}), Method::POST, "/api/v1/feedback"),
        ("docbrain_revise", json!({"id": ID, "withdraw_reason": "gone"}), Method::POST, "/api/v1/fragments/0b9c3f5e-6a1d-4d8e-9f2a-3c4b5d6e7f80/withdraw"),
    ];
    for (tool, args, method, path) in cases {
        let before = backend.sent().len();
        let (result, error) = call(&server, tool, args).await;
        assert_eq!(error, None, "{tool} answered a protocol error");
        assert_ne!(result.as_ref().map(|r| r["isError"] == json!(true)), Some(true), "{tool}: {:?}", result);
        let sent = backend.sent();
        let first = &sent[before];
        assert_eq!((first.method.clone(), first.path.as_str()), (method, path), "{tool}");
    }
    let freshness = backend.sent().into_iter().find(|r| r.path.as_str() == "/api/v1/freshness").expect("freshness");
    assert_eq!(freshness.query, vec![("space", "ops docs".to_string())], "the space is a query value, encoded by the backend");
}

#[tokio::test]
async fn the_gap_tools_and_generate_send_their_requests_through_the_backend() {
    let backend = Recording::answering(|r| {
        let body = match r.path.as_str() {
            "/api/v1/autopilot/gaps" => json!([]),
            _ => json!({"draft_id": ID, "title": "t", "content": "c"}),
        };
        Ok(ApiResponse { status: 200, body: body.to_string() })
    });
    let server = McpServer::with_backend(backend.clone());
    call(&server, "docbrain_autopilot_gaps", json!({"limit": 500})).await;
    call(&server, "docbrain_suggest_capture", json!({"file_path": "src/a.rs"})).await;
    call(&server, "docbrain_autopilot_generate", json!({"cluster_id": ID})).await;
    let sent: Vec<(Method, String, Vec<(&str, String)>)> =
        backend.sent().into_iter().map(|r| (r.method, r.path.as_str().to_string(), r.query)).collect();
    assert_eq!(
        sent,
        vec![
            (Method::GET, "/api/v1/autopilot/gaps".to_string(), vec![("limit", "200".to_string())]),
            (Method::GET, "/api/v1/autopilot/gaps".to_string(), vec![("limit", "50".to_string())]),
            (Method::POST, format!("/api/v1/autopilot/generate/{ID}"), vec![]),
            (Method::GET, format!("/api/v1/autopilot/drafts/{ID}"), vec![]),
        ]
    );
}

#[tokio::test]
async fn only_ask_waits_without_a_client_timeout() {
    let backend = Recording::ok(json!({"answer": "a", "sources": []}));
    let server = McpServer::with_backend(backend.clone());
    call(&server, "docbrain_ask", json!({"question": "q"})).await;
    call(&server, "docbrain_incident", json!({"description": "d"})).await;
    let sent = backend.sent();
    assert!(sent[0].long, "ask may outlast the default client timeout (#443)");
    assert!(!sent[1].long);
}

#[tokio::test]
async fn a_refusal_from_the_server_is_a_tool_error_not_a_protocol_error() {
    let backend = Recording::answering(|_| Ok(ApiResponse { status: 503, body: "busy".into() }));
    let server = McpServer::with_backend(backend);
    let (result, error) = call(&server, "docbrain_incident", json!({"description": "d"})).await;
    assert_eq!(error, None);
    let result = result.expect("a result");
    assert_eq!(result["isError"], json!(true));
    assert!(text(&result).contains("503"), "{}", text(&result));
}

#[tokio::test]
async fn an_unreachable_server_is_a_tool_error() {
    let backend = Recording::answering(|_| Err("connection refused".into()));
    let server = McpServer::with_backend(backend);
    let (result, error) = call(&server, "docbrain_autopilot_summary", json!({})).await;
    assert_eq!(error, None);
    assert_eq!(result.expect("a result")["isError"], json!(true));
}

#[tokio::test]
async fn an_id_that_is_not_a_uuid_never_reaches_a_path() {
    let backend = Recording::ok(json!({}));
    let server = McpServer::with_backend(backend.clone());
    for id in ["x/../../fragments/abc/withdraw", "", "not-a-uuid", "0b9c3f5e-6a1d-4d8e-9f2a-3c4b5d6e7f80/../x"] {
        let (result, error) = call(&server, "docbrain_autopilot_generate", json!({"cluster_id": id})).await;
        assert_eq!(error, None, "{id:?}");
        assert_eq!(result.expect("a result")["isError"], json!(true), "{id:?}");
    }
    assert!(backend.sent().is_empty(), "no request may carry an unchecked id");
}

#[tokio::test]
async fn a_draft_id_from_the_server_goes_into_a_path_only_when_it_is_an_id() {
    let backend = Recording::ok(json!({"draft_id": "../../fragments/x/withdraw", "title": "t"}));
    let server = McpServer::with_backend(backend.clone());
    let (result, error) = call(&server, "docbrain_autopilot_generate", json!({"cluster_id": ID})).await;
    assert_eq!(error, None);
    assert_ne!(result.expect("a result")["isError"], json!(true), "the draft was generated; only its text is not fetched");
    let paths: Vec<String> = backend.sent().iter().map(|r| r.path.as_str().to_string()).collect();
    assert_eq!(paths, vec![format!("/api/v1/autopilot/generate/{ID}")]);
}

#[tokio::test]
async fn a_write_tool_is_decided_by_the_server_not_by_a_cached_role() {
    // No role was ever looked up: the server's answer alone decides.
    let backend = Recording::answering(|_| Ok(ApiResponse { status: 201, body: json!({"id": ID, "status": "indexed"}).to_string() }));
    let server = McpServer::with_backend(backend.clone());
    let (result, error) = call(&server, "docbrain_annotate", json!({"file_path": "a.rs", "annotation": "why"})).await;
    assert_eq!(error, None);
    assert_ne!(result.expect("a result")["isError"], json!(true));
    assert_eq!(backend.sent().len(), 1);
}

/// rev 8 §4: a route refusal (role, secret gate, duplicate) is a tool result with `isError: true`, in
/// the server's words — never a plain result a host or model could read as saved. The bodies are the
/// server's own refusal shapes (routes/capture_refusal.rs, admin_auth.rs, fragments/lifecycle.rs).
#[tokio::test]
async fn a_refused_capture_edit_or_withdraw_is_a_tool_error_in_the_servers_words() {
    let refusing = |status: u16, body: Value| {
        let body = body.to_string();
        Recording::answering(move |_| Ok(ApiResponse { status, body: body.clone() }))
    };
    let viewer = json!({"error": "Editor role required", "kind": "invalid_request"});
    let annotate = json!({"file_path": "a.rs", "annotation": "why"});
    let cases = [
        ("docbrain_annotate", annotate.clone(), 403, viewer.clone(), "Not saved (403): Editor role required".to_string()),
        ("docbrain_annotate", annotate.clone(), 409, json!({"error": format!("already captured: {ID}"), "kind": "already_captured", "id": ID}),
            format!("Already saved: {ID}. DocBrain already has this capture; use docbrain_revise to change it.")),
        // Another capture at the same anchor (rev 8 §4.2, B6): its id, so the person can be offered a revise.
        ("docbrain_annotate", annotate, 409, json!({"error": "a capture with this source_id already exists", "kind": "duplicate_source", "id": ID}),
            format!("Not saved (409): a capture with this source_id already exists (id {ID}); use docbrain_revise on it to change that capture.")),
        ("docbrain_commit_capture", json!({"intent": "why"}), 403, viewer, "Not saved (403): Editor role required".to_string()),
        ("docbrain_revise", json!({"id": ID, "withdraw_reason": "gone"}), 409, json!({"error": "this capture is already withdrawn", "kind": "already_withdrawn"}),
            "Not changed (409): this capture is already withdrawn".to_string()),
    ];
    for (tool, args, status, body, words) in cases {
        let server = McpServer::with_backend(refusing(status, body));
        let (result, error) = call(&server, tool, args).await;
        assert_eq!(error, None, "{tool} {status}: a tool result, not a protocol error");
        let result = result.expect("a result");
        assert_eq!(result["isError"], json!(true), "{tool} {status}: {result}");
        assert_eq!(text(&result), words, "{tool} {status}");
    }
}

#[tokio::test]
async fn a_missing_argument_is_a_tool_error() {
    let backend = Recording::ok(json!({}));
    let server = McpServer::with_backend(backend.clone());
    let (result, error) = call(&server, "docbrain_annotate", json!({"annotation": "why"})).await;
    assert_eq!(error, None);
    let result = result.expect("a result");
    assert_eq!(result["isError"], json!(true));
    assert!(text(&result).contains("file_path"), "{}", text(&result));
    assert!(backend.sent().is_empty());
}

#[tokio::test]
async fn an_unknown_tool_is_a_protocol_error() {
    let server = McpServer::with_backend(Recording::ok(json!({})));
    let (result, error) = call(&server, "docbrain_nothing", json!({})).await;
    assert_eq!((result, error), (None, Some(-32602)));
}
