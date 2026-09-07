// SPDX-License-Identifier: MIT
//! The shim must carry the claim-verification verdict to the MCP host.
//!
//! `tool_ask` built its text from `answer`, `sources` and `episode_id` only, so
//! the verdict the server computed was discarded before Claude or Cursor ever
//! saw it. On those surfaces an answer asserting a path that exists nowhere
//! arrived with no warning at all — while the web UI and the CLI both showed
//! one. This is the producer half of that contract; the wording itself comes
//! from the server's `notes`, never re-derived here, so all three surfaces say
//! the same thing.

use axum::{routing::post, Json, Router};
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use tokio::net::TcpListener;

/// The canned `/api/v1/ask` and `/api/v1/incident` body this mock returns.
type Canned = Arc<Mutex<Value>>;

async fn handler(axum::extract::State(body): axum::extract::State<Canned>) -> Json<Value> {
    Json(body.lock().expect("canned body").clone())
}

async fn spawn_mock(canned: Value) -> (Canned, String) {
    let state: Canned = Arc::new(Mutex::new(canned));
    let app = Router::new()
        .route("/api/v1/ask", post(handler))
        .route("/api/v1/incident", post(handler))
        .route("/api/v1/fragments/context", post(handler))
        .with_state(state.clone());
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("bind ephemeral port");
    let addr = listener.local_addr().expect("local_addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum serve");
    });
    (state, format!("http://{}", addr))
}

/// Drive a tool through the public JSON-RPC entry and return `content[0].text`.
async fn call_tool(base_url: &str, tool: &str, args: Value) -> String {
    let server = McpServer::from_parts(base_url.to_string(), Some("test-key".to_string()));
    let resp = server
        .handle_request(JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: "tools/call".into(),
            params: json!({ "name": tool, "arguments": args }),
        })
        .await;
    assert!(resp.error.is_none(), "tool errored: {:?}", resp.error);
    resp.result.expect("result")["content"][0]["text"]
        .as_str()
        .expect("text content")
        .to_string()
}

fn failed_body(answer: &str) -> Value {
    json!({
        "answer": answer,
        "sources": [],
        "verification": {
            "status": "failed",
            "checked": 1,
            "failures": [{
                "text": "clusters/dev/apps/kustomization.yaml",
                "verdict": { "verdict": "missing" }
            }],
            "notes": ["⚠ Unverified: the documentation says `clusters/dev/apps/kustomization.yaml`, which does not exist in the connected sources."]
        }
    })
}

#[tokio::test]
async fn ask_carries_a_failed_verdict_above_the_answer() {
    let (_c, url) = spawn_mock(failed_body("Reconciles from clusters/dev/apps.")).await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "where?"})).await;

    assert!(text.contains("⚠ Unverified"), "verdict absent: {text}");
    assert!(
        text.contains("clusters/dev/apps/kustomization.yaml"),
        "the claim was not named: {text}"
    );
    // Above the answer, mirroring the web banner, which sits above the
    // confidence line deliberately: burying a failed check under the answer
    // inverts which signal is stronger, and a host reading top-down is least
    // able to paraphrase away what it reads first.
    let warn_at = text.find("⚠ Unverified").expect("warning present");
    let answer_at = text.find("Reconciles from").expect("answer present");
    assert!(warn_at < answer_at, "warning sits below the answer: {text}");
}

#[tokio::test]
async fn ask_reports_a_passed_verdict_once_and_quietly() {
    let (_c, url) = spawn_mock(json!({
        "answer": "All good.",
        "sources": [],
        "verification": { "status": "passed", "checked": 2, "failures": [], "notes": [] }
    }))
    .await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "where?"})).await;

    assert!(text.contains("2"), "checked count absent: {text}");
    assert!(!text.contains("⚠"), "a pass must not carry a warning glyph: {text}");
}

/// `nothing_checkable` is the outcome for EVERY answer on a deployment whose
/// sources record no file listings. A line on each one would be indistinguishable
/// from a broken feature — the same reason the web banner renders nothing here.
#[tokio::test]
async fn ask_says_nothing_when_nothing_was_checkable() {
    let (_c, url) = spawn_mock(json!({
        "answer": "Conceptual answer.",
        "sources": [],
        "verification": { "status": "nothing_checkable", "checked": 0, "failures": [], "notes": [] }
    }))
    .await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "why?"})).await;

    assert_eq!(text.trim(), "Conceptual answer.", "added noise: {text}");
}

/// An absent `verification` means the pass never ran (no oracle configured).
/// Reporting a verdict that was never computed is the failure mode
/// `Answer::verification` being an Option exists to prevent.
#[tokio::test]
async fn ask_invents_no_verdict_when_the_pass_did_not_run() {
    let (_c, url) = spawn_mock(json!({ "answer": "Plain answer.", "sources": [] })).await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "why?"})).await;

    assert_eq!(text.trim(), "Plain answer.", "synthesised a verdict: {text}");
}

/// Incident answers are the highest-stakes place for a fabricated path: someone
/// is mid-incident following what this returns.
#[tokio::test]
async fn incident_carries_a_failed_verdict_too() {
    let (_c, url) = spawn_mock(failed_body("Follow clusters/dev/apps.")).await;
    let text = call_tool(&url, "docbrain_incident", json!({"description": "pods crashlooping"})).await;

    assert!(text.contains("⚠ Unverified"), "verdict absent on incident: {text}");
    let warn_at = text.find("⚠ Unverified").expect("warning present");
    let answer_at = text.find("Follow clusters").expect("answer present");
    assert!(warn_at < answer_at, "warning sits below the answer: {text}");
}

/// A stale claim must reach the MCP host, above the answer. The shim owns no
/// pixels — whatever the host reads first is what it is least able to
/// paraphrase away.
#[tokio::test]
async fn ask_carries_a_stale_claim_above_the_answer() {
    let (_c, url) = spawn_mock(json!({
        "answer": "The chart is pinned to 1.4.0.",
        "sources": [],
        "stale_claims": [{
            "fragment_id": "00000000-0000-0000-0000-000000000001",
            "expression": "acme/Chart.yaml#version=1.4.0",
            "asserted": "1.4.0",
            "current": "9.9.9",
            "basis": "local:acme (captured 2026-09-05)",
            "note": "⚠ Out of date: this answer draws on a note asserting `acme/Chart.yaml#version=1.4.0`; the source now says `9.9.9` (local:acme (captured 2026-09-05))."
        }]
    })).await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "which version?"})).await;

    assert!(text.contains("⚠ Out of date"), "stale claim absent: {text}");
    assert!(text.contains("9.9.9"), "the current value must be named: {text}");
    let warn = text.find("⚠ Out of date").expect("warning present");
    let answer = text.find("The chart is pinned").expect("answer present");
    assert!(warn < answer, "warning sits below the answer: {text}");
}

/// A stale claim in an incident response must also land above the header —
/// mirroring `ask_carries_a_stale_claim_above_the_answer` and
/// `incident_carries_a_failed_verdict_too`, which cover the verdict half of
/// this same surface. `tool_incident` builds `stale` (`lib.rs`) but a prior
/// gap meant nothing asserted it was ever interpolated: deleting `{stale}`
/// from that format string left the whole suite green.
#[tokio::test]
async fn incident_carries_a_stale_claim_above_the_header() {
    let (_c, url) = spawn_mock(json!({
        "answer": "Roll back the chart to 1.4.0.",
        "sources": [],
        "stale_claims": [{
            "fragment_id": "00000000-0000-0000-0000-000000000002",
            "expression": "acme/Chart.yaml#version=1.4.0",
            "asserted": "1.4.0",
            "current": "9.9.9",
            "basis": "local:acme (captured 2026-09-05)",
            "note": "⚠ Out of date: this answer draws on a note asserting `acme/Chart.yaml#version=1.4.0`; the source now says `9.9.9` (local:acme (captured 2026-09-05))."
        }]
    })).await;
    let text = call_tool(&url, "docbrain_incident", json!({"description": "chart rollback"})).await;

    assert!(text.contains("⚠ Out of date"), "stale claim absent from incident: {text}");
    let warn = text.find("⚠ Out of date").expect("warning present");
    let header = text.find("**INCIDENT RESPONSE**").expect("header present");
    assert!(warn < header, "the stale claim must sit above the incident header: {text}");
}

/// No stale claims must add nothing at all — a line on every response is
/// indistinguishable from a broken feature.
#[tokio::test]
async fn ask_adds_nothing_when_no_claim_is_stale() {
    let (_c, url) = spawn_mock(json!({ "answer": "All current.", "sources": [] })).await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "q"})).await;
    assert_eq!(text.trim(), "All current.", "added noise: {text}");
}

/// The server decides what the reader needs — the shim must not gate on a
/// closed set of known `kind`s. A `kind` this build has never seen must still
/// reach the host, in the server's order, above the answer.
#[tokio::test]
async fn an_unknown_block_kind_still_reaches_the_host() {
    let (_c, url) = spawn_mock(json!({
        "answer": "the answer",
        "blocks": [
            { "kind": "something-new", "text": "a future warning" },
            { "kind": "answer", "text": "the answer" }
        ]
    }))
    .await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "q"})).await;

    let warn = text.find("a future warning").expect("unknown kind must still render");
    let answer = text.find("the answer").expect("answer present");
    assert!(warn < answer, "warning sits below the answer: {text}");
}

/// The answer block must NOT be echoed above the answer the host already gets.
///
/// The fixture above contains an `answer` block, which implies this is covered
/// — it is not: `find` returns the FIRST occurrence, so a second copy further
/// down changes nothing it asserts. Deleting the `kind == "answer"` filter left
/// the whole suite green while every host saw the answer twice.
#[tokio::test]
async fn the_answer_block_is_not_repeated_above_the_answer() {
    let (_c, url) = spawn_mock(json!({
        "answer": "the answer",
        "blocks": [
            { "kind": "warning", "text": "a stale note" },
            { "kind": "answer", "text": "the answer" }
        ]
    }))
    .await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "q"})).await;
    assert_eq!(text.matches("the answer").count(), 1, "the answer was repeated: {text}");
}

/// Order is the server's. The unknown-kind fixture has a single non-answer
/// block, so reversing or sorting the list produces byte-identical output and
/// cannot fail it. Two warnings can.
#[tokio::test]
async fn leading_lines_keep_the_servers_order() {
    let (_c, url) = spawn_mock(json!({
        "answer": "the answer",
        "blocks": [
            { "kind": "warning", "text": "zebra first" },
            { "kind": "warning", "text": "alpha second" },
            { "kind": "answer", "text": "the answer" }
        ]
    }))
    .await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "q"})).await;
    let z = text.find("zebra first").expect("first warning present");
    let a = text.find("alpha second").expect("second warning present");
    assert!(z < a, "the shim re-ordered the server's blocks: {text}");
}

/// A text-less block must vanish rather than become a blank line, and a blocks
/// array carrying nothing but the answer must add nothing at all.
#[tokio::test]
async fn empty_blocks_add_no_blank_lines() {
    let (_c, url) = spawn_mock(json!({
        "answer": "All current.",
        "blocks": [
            { "kind": "note", "text": "" },
            { "kind": "answer", "text": "All current." }
        ]
    }))
    .await;
    let text = call_tool(&url, "docbrain_ask", json!({"question": "q"})).await;
    assert_eq!(text.trim(), "All current.", "a text-less block leaked whitespace: {text:?}");
    assert!(!text.starts_with('\n'), "leading blank line before the answer: {text:?}");
}

/// The incident path had a stale-claim test but no blocks test, so reverting
/// `tool_incident` to a `stale_claims`-only reader left the suite green — the
/// server still sends both fields.
#[tokio::test]
async fn incident_renders_blocks_above_the_header() {
    let (_c, url) = spawn_mock(json!({
        "answer": "Restart the pod.",
        "blocks": [
            { "kind": "something-new", "text": "a future warning" },
            { "kind": "answer", "text": "Restart the pod." }
        ]
    }))
    .await;
    let text = call_tool(&url, "docbrain_incident", json!({"description": "pods crashlooping"})).await;
    let warn = text.find("a future warning").expect("blocks must reach the incident path");
    let header = text.find("**INCIDENT RESPONSE**").expect("header present");
    assert!(warn < header, "warning sits below the header: {text}");
}

/// The whole point of the tool: a note bound to a file reaches the agent, with
/// any stale-premise warning above it, in the server's order.
#[tokio::test]
async fn context_returns_warnings_above_notes() {
    let (_c, url) = spawn_mock(json!({
        "blocks": [
            { "kind": "warning", "text": "⚠ Out of date: acme", "severity": "stale" },
            { "kind": "note", "text": "we cannot upgrade this chart yet" }
        ],
        "checked_paths": ["acme/svc/main.rs"],
        "fragments_found": 1
    })).await;
    let text = call_tool(&url, "docbrain_context", json!({"file_paths": ["acme/svc/main.rs"]})).await;
    let warn = text.find("⚠ Out of date").expect("warning present");
    let note = text.find("we cannot upgrade").expect("note present");
    assert!(warn < note, "warning sits below the note: {text}");
}

/// Nothing captured is NOT silence. A tool that returns an empty string lets
/// the agent conclude the organisation has no opinion, which is the same
/// failure as an over-filtered query reading as "all is well".
#[tokio::test]
async fn context_says_it_checked_when_it_found_nothing() {
    let (_c, url) = spawn_mock(json!({
        "blocks": [], "checked_paths": ["acme/svc/main.rs"], "fragments_found": 0
    })).await;
    let text = call_tool(&url, "docbrain_context", json!({"file_paths": ["acme/svc/main.rs"]})).await;
    assert!(text.contains("acme/svc/main.rs"), "must name what it checked: {text}");
    assert!(!text.trim().is_empty(), "an empty string reads as 'no opinion'");
}

/// A kind this build has never seen must still reach the host.
#[tokio::test]
async fn context_renders_an_unknown_block_kind() {
    let (_c, url) = spawn_mock(json!({
        "blocks": [{ "kind": "something-new", "text": "a future signal" }],
        "checked_paths": ["acme/svc/main.rs"], "fragments_found": 1
    })).await;
    let text = call_tool(&url, "docbrain_context", json!({"file_paths": ["acme/svc/main.rs"]})).await;
    assert!(text.contains("a future signal"), "unknown kinds must render: {text}");
}
