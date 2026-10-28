// SPDX-License-Identifier: MIT
//! The MCP Streamable HTTP binding's decision table (#601 rev 8 §4.1): one test per row.
//! Modern requests (revision 2026-07-28) carry their version in `_meta` and mirror it in
//! headers; legacy requests (2025-11-25, 2025-06-18) open with a headerless `initialize`.

use docbrain_mcp::backend::{ApiRequest, ApiResponse, Backend};
use docbrain_mcp::http::{prepare, Header, HttpView, Prepared, LEGACY_VERSIONS, MODERN_VERSION};
use docbrain_mcp::McpServer;
use serde_json::{json, Value};

struct Unused;
impl Backend for Unused {
    async fn send(&self, _: ApiRequest) -> Result<ApiResponse, String> {
        Err("no request expected while preparing".into())
    }
}

fn server() -> McpServer<Unused> {
    McpServer::with_backend(Unused)
}

fn modern_meta() -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": MODERN_VERSION,
        "io.modelcontextprotocol/clientInfo": {"name": "t", "version": "1"},
        "io.modelcontextprotocol/clientCapabilities": {}
    })
}

fn modern(method: &str, mut params: Value) -> Value {
    params["_meta"] = modern_meta();
    json!({"jsonrpc": "2.0", "id": 7, "method": method, "params": params})
}

struct Req {
    version: Header<'static>,
    method: Header<'static>,
    name: Header<'static>,
    body: Vec<u8>,
}

impl Req {
    fn new(body: Value) -> Self {
        Self { version: Header::Absent, method: Header::Absent, name: Header::Absent, body: body.to_string().into_bytes() }
    }
    fn modern(method: &'static str, params: Value) -> Self {
        let mut r = Self::new(modern(method, params));
        r.version = Header::One(MODERN_VERSION);
        r.method = Header::One(method);
        r
    }
    fn prepared(&self) -> Prepared {
        prepare(&server(), HttpView { protocol_version: self.version, mcp_method: self.method, mcp_name: self.name, body: &self.body })
    }
}

fn reply(p: Prepared) -> (u16, Option<Value>) {
    match p {
        Prepared::Reply { status, body } => (status, body),
        Prepared::Call(c) => panic!("expected a reply, got a call to {}", c.name),
    }
}

fn error_code(body: &Option<Value>) -> i64 {
    body.as_ref().and_then(|b| b["error"]["code"].as_i64()).unwrap_or(0)
}

// ── modern (2026-07-28) ──────────────────────────────────────────────────────────────────────

#[test]
fn modern_tools_list_is_cacheable_and_names_the_server() {
    let (status, body) = reply(Req::modern("tools/list", json!({})).prepared());
    let r = &body.expect("body")["result"];
    assert_eq!(status, 200);
    assert!(r["tools"].as_array().is_some_and(|t| !t.is_empty()));
    assert_eq!(r["resultType"], "complete");
    assert!(r["ttlMs"].as_u64().is_some());
    assert_eq!(r["cacheScope"], "private");
    assert_eq!(r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "docbrain");
}

#[test]
fn modern_server_discover_lists_every_supported_version() {
    let (status, body) = reply(Req::modern("server/discover", json!({})).prepared());
    let r = &body.expect("body")["result"];
    assert_eq!(status, 200);
    let mut expected = vec![MODERN_VERSION];
    expected.extend(LEGACY_VERSIONS);
    assert_eq!(r["supportedVersions"], json!(expected));
    assert!(r["capabilities"]["tools"].is_object());
    assert_eq!(r["resultType"], "complete");
    assert!(r["ttlMs"].as_u64().is_some());
    assert!(r["cacheScope"].is_string());
    assert_eq!(r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "docbrain");
}

#[test]
fn modern_tools_call_with_a_matching_name_header_is_run_and_answered_in_the_modern_envelope() {
    let mut r = Req::modern("tools/call", json!({"name": "docbrain_ask", "arguments": {"question": "q"}}));
    r.name = Header::One("docbrain_ask");
    let Prepared::Call(call) = r.prepared() else { panic!("expected a call") };
    assert_eq!((call.name.as_str(), &call.arguments), ("docbrain_ask", &json!({"question": "q"})));
    let answer = call.answer(Ok(json!({"content": []})));
    assert_eq!(answer["id"], 7);
    assert_eq!(answer["result"]["resultType"], "complete");
    assert_eq!(answer["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "docbrain");
}

#[test]
fn modern_tools_call_accepts_a_base64_encoded_name_header() {
    let mut r = Req::modern("tools/call", json!({"name": "docbrain_ask", "arguments": {}}));
    r.name = Header::One("=?base64?ZG9jYnJhaW5fYXNr?=");
    assert!(matches!(r.prepared(), Prepared::Call(_)));
}

#[test]
fn modern_tools_call_refuses_a_missing_wrong_or_undecodable_name_header() {
    for name in [Header::Absent, Header::One("docbrain_incident"), Header::One("=?base64?***?="), Header::Invalid] {
        let mut r = Req::modern("tools/call", json!({"name": "docbrain_ask", "arguments": {}}));
        r.name = name;
        let (status, body) = reply(r.prepared());
        assert_eq!((status, error_code(&body)), (400, -32020), "{name:?}");
    }
}

#[test]
fn modern_method_header_must_be_present_and_match() {
    for method in [Header::Absent, Header::One("tools/call"), Header::Invalid] {
        let mut r = Req::modern("tools/list", json!({}));
        r.method = method;
        let (status, body) = reply(r.prepared());
        assert_eq!((status, error_code(&body)), (400, -32020), "{method:?}");
    }
}

#[test]
fn a_modern_header_needs_the_same_version_in_meta() {
    let mut without_meta = Req::new(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {}}));
    without_meta.version = Header::One(MODERN_VERSION);
    without_meta.method = Header::One("tools/list");
    assert_eq!(error_code(&reply(without_meta.prepared()).1), -32020);

    let mut other = Req::modern("tools/list", json!({}));
    let mut body: Value = serde_json::from_slice(&other.body).expect("json");
    body["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2025-11-25");
    other.body = body.to_string().into_bytes();
    let (status, b) = reply(other.prepared());
    assert_eq!((status, error_code(&b)), (400, -32020));
}

#[test]
fn modern_meta_without_client_capabilities_is_invalid_params() {
    let mut r = Req::modern("tools/list", json!({}));
    let mut body: Value = serde_json::from_slice(&r.body).expect("json");
    body["params"]["_meta"].as_object_mut().expect("meta").remove("io.modelcontextprotocol/clientCapabilities");
    r.body = body.to_string().into_bytes();
    let (status, b) = reply(r.prepared());
    assert_eq!((status, error_code(&b)), (400, -32602));
}

#[test]
fn a_modern_request_for_a_removed_or_unknown_method_is_not_found() {
    for method in ["ping", "initialize", "resources/list", "logging/setLevel"] {
        let (status, body) = reply(Req::modern(method, json!({})).prepared());
        assert_eq!((status, error_code(&body)), (404, -32601), "{method}");
    }
}

#[test]
fn a_repeated_or_non_ascii_version_header_is_a_header_mismatch() {
    let mut r = Req::modern("tools/list", json!({}));
    r.version = Header::Invalid;
    let (status, body) = reply(r.prepared());
    assert_eq!((status, error_code(&body)), (400, -32020));
}

#[test]
fn a_modern_notification_is_accepted_without_a_body() {
    let mut body = modern("notifications/whatever", json!({}));
    body.as_object_mut().expect("obj").remove("id");
    let mut r = Req::new(body);
    r.version = Header::One(MODERN_VERSION);
    r.method = Header::One("notifications/whatever");
    assert_eq!(reply(r.prepared()), (202, None));
}

// ── legacy (2025-11-25, 2025-06-18) ─────────────────────────────────────────────────────────

fn legacy(version: &'static str, body: Value) -> Req {
    let mut r = Req::new(body);
    r.version = Header::One(version);
    r
}

#[test]
fn a_headerless_initialize_negotiates_the_requested_legacy_version() {
    for v in LEGACY_VERSIONS {
        let r = Req::new(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": v, "capabilities": {}, "clientInfo": {"name": "t", "version": "1"}}}));
        let (status, body) = reply(r.prepared());
        let result = &body.expect("body")["result"];
        assert_eq!(status, 200);
        assert_eq!(result["protocolVersion"], v);
        assert!(result["capabilities"]["tools"].is_object());
        assert_eq!(result["serverInfo"]["name"], "docbrain");
        assert!(result.get("resultType").is_none(), "legacy results keep the legacy shape");
    }
}

#[test]
fn a_headerless_initialize_for_an_unserved_version_gets_the_newest_legacy_one() {
    let r = Req::new(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}));
    assert_eq!(reply(r.prepared()).1.expect("body")["result"]["protocolVersion"], LEGACY_VERSIONS[0]);
}

#[test]
fn a_notification_without_a_version_header_is_accepted() {
    let r = Req::new(json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    assert_eq!(reply(r.prepared()), (202, None));
}

#[test]
fn anything_but_initialize_without_a_version_header_is_an_unsupported_version() {
    let r = Req::new(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
    let (status, body) = reply(r.prepared());
    let body = body.expect("body");
    assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(-32022)));
    assert!(body["error"]["data"]["supported"].as_array().is_some_and(|s| s.contains(&json!(MODERN_VERSION))));
}

#[test]
fn legacy_tools_list_keeps_the_legacy_shape() {
    for v in LEGACY_VERSIONS {
        let (status, body) = reply(legacy(v, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})).prepared());
        let result = &body.expect("body")["result"];
        assert_eq!(status, 200);
        assert!(result["tools"].as_array().is_some_and(|t| !t.is_empty()));
        assert!(result.get("resultType").is_none());
    }
}

#[test]
fn legacy_notifications_are_accepted_without_a_body() {
    for method in ["notifications/initialized", "notifications/cancelled"] {
        let r = legacy(LEGACY_VERSIONS[1], json!({"jsonrpc": "2.0", "method": method, "params": {}}));
        assert_eq!(reply(r.prepared()), (202, None), "{method}");
    }
}

#[test]
fn legacy_ping_is_answered() {
    let (status, body) = reply(legacy(LEGACY_VERSIONS[0], json!({"jsonrpc": "2.0", "id": 3, "method": "ping"})).prepared());
    assert_eq!((status, body.expect("body")["result"].clone()), (200, json!({})));
}

#[test]
fn legacy_tools_call_is_run_and_answered_in_the_legacy_shape() {
    let r = legacy(LEGACY_VERSIONS[0], json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "docbrain_ask", "arguments": {"question": "q"}}}));
    let Prepared::Call(call) = r.prepared() else { panic!("expected a call") };
    let answer = call.answer(Ok(json!({"content": []})));
    assert_eq!(answer["id"], 4);
    assert!(answer["result"].get("resultType").is_none());
}

#[test]
fn a_legacy_request_carrying_modern_meta_is_served_as_legacy() {
    let mut body = modern("tools/list", json!({}));
    body["id"] = json!(5);
    let (status, b) = reply(legacy(LEGACY_VERSIONS[0], body).prepared());
    let b = b.expect("body");
    assert_eq!(status, 200);
    assert!(b["result"]["tools"].is_array());
    assert!(b["result"].get("resultType").is_none());
}

#[test]
fn an_unserved_version_header_is_refused_with_the_supported_list() {
    for v in ["2024-11-05", "2025-03-26", "1900-01-01", "garbage"] {
        let (status, body) = reply(legacy(v, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).prepared());
        let body = body.expect("body");
        assert_eq!((status, body["error"]["code"].as_i64()), (400, Some(-32022)), "{v}");
        assert_eq!(body["error"]["data"]["requested"], v);
    }
}

#[test]
fn a_legacy_unknown_method_is_a_json_rpc_error() {
    let (status, body) = reply(legacy(LEGACY_VERSIONS[0], json!({"jsonrpc": "2.0", "id": 6, "method": "resources/list"})).prepared());
    assert_eq!((status, error_code(&body)), (200, -32601));
}

// ── malformed ───────────────────────────────────────────────────────────────────────────────

#[test]
fn a_batch_is_refused() {
    let r = legacy(LEGACY_VERSIONS[0], json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]));
    let (status, body) = reply(r.prepared());
    assert_eq!(status, 400);
    assert!(body.and_then(|b| b.get("id").cloned()).is_none(), "an id that could not be read is left out, never null");
}

#[test]
fn a_body_that_is_not_json_is_a_parse_error() {
    let mut r = Req::new(json!(null));
    r.body = b"{not json".to_vec();
    let (status, body) = reply(r.prepared());
    assert_eq!((status, error_code(&body)), (400, -32700));
    assert!(body.and_then(|b| b.get("id").cloned()).is_none(), "an MCP id is never null");
}
