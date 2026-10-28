// SPDX-License-Identifier: MIT
//! MCP's Streamable HTTP binding, as DocBrain's own `/api/v1/mcp` serves it (#601 rev 8 §4.1).
//!
//! The server is "dual-era": a modern request (revision 2026-07-28) carries its protocol version
//! in `_meta` and mirrors it, the method and the tool name in headers, which must agree with the
//! body; a legacy client (2025-11-25, 2025-06-18) opens with an `initialize` that carries no
//! version header and names its version in every later request's header. Every request is
//! answered on its own: there are no sessions.
//!
//! [`prepare`] reads one request and either answers it at once or hands back the tool call to
//! run; [`ToolCall::answer`] wraps the tool's result for the request's era. The HTTP framing
//! (status codes on the wire, SSE, authentication) is the server's.

use base64::Engine;
use serde_json::{Value, json};

use crate::backend::Backend;
use crate::{JsonRpcError, McpServer, SERVER_NAME, SERVER_VERSION};

/// The modern revision this endpoint speaks.
pub const MODERN_VERSION: &str = "2026-07-28";
/// The legacy revisions it serves, newest first. 2025-03-26 is not offered over HTTP: it
/// requires batches and has no version header (#601 rev 8 §4.1).
pub const LEGACY_VERSIONS: [&str; 2] = ["2025-11-25", "2025-06-18"];

const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const META_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";
const META_SERVER: &str = "io.modelcontextprotocol/serverInfo";
/// How long a client may reuse the tool list: tools change only with a server upgrade.
const TOOLS_TTL_MS: u64 = 300_000;
const DISCOVER_TTL_MS: u64 = 3_600_000;

/// One HTTP header as the binding needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Header<'a> {
    Absent,
    One(&'a str),
    /// Repeated, or not plain visible ASCII.
    Invalid,
}

/// The parts of a POST to the MCP endpoint that the binding reads.
pub struct HttpView<'a> {
    pub protocol_version: Header<'a>,
    pub mcp_method: Header<'a>,
    pub mcp_name: Header<'a>,
    pub body: &'a [u8],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Era {
    Modern,
    Legacy,
}

/// What to do with a request.
pub enum Prepared {
    /// Answer now with this HTTP status and JSON body (none: 202 for a notification).
    Reply { status: u16, body: Option<Value> },
    /// Run this tool, then answer with [`ToolCall::answer`].
    Call(ToolCall),
}

/// A `tools/call` the server runs.
pub struct ToolCall {
    pub id: Value,
    pub name: String,
    pub arguments: Value,
    era: Era,
}

impl ToolCall {
    /// The JSON-RPC response for the tool's outcome, in this request's era.
    pub fn answer(&self, outcome: Result<Value, JsonRpcError>) -> Value {
        match outcome {
            Ok(result) => response(&self.id, envelope(self.era, result)),
            Err(e) => error_response(&self.id, e.code, &e.message, None),
        }
    }
}

/// Read one request (§4.1's decision table, after Origin and authentication).
pub fn prepare<B: Backend>(server: &McpServer<B>, view: HttpView) -> Prepared {
    let message: Value = match serde_json::from_slice(view.body) {
        Ok(v) => v,
        Err(e) => return reply(400, error_response(&Value::Null, -32700, &format!("Parse error: {e}"), None)),
    };
    let Some(object) = message.as_object() else {
        return reply(
            400,
            error_response(&Value::Null, -32600, "Invalid request: one JSON-RPC message per request, no batches", None),
        );
    };
    let id = object.get("id").cloned();
    let reply_id = id.clone().unwrap_or(Value::Null);
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        return reply(400, error_response(&reply_id, -32600, "Invalid request: no method", None));
    };
    let params = object.get("params").cloned().unwrap_or(Value::Null);
    let meta_version = params.get("_meta").and_then(|m| m.get(META_VERSION)).and_then(Value::as_str);
    let notification = id.is_none();

    match view.protocol_version {
        Header::Invalid => mismatch(&reply_id, "the MCP-Protocol-Version header is repeated or not plain ASCII"),
        Header::One(MODERN_VERSION) => modern(server, &view, reply_id, notification, method, &params, meta_version),
        Header::One(v) if LEGACY_VERSIONS.contains(&v) => legacy(server, reply_id, notification, method, &params),
        // A legacy client's first request: the version is named in the body, not in a header.
        Header::Absent if method == "initialize" && meta_version.is_none() => {
            reply(200, response(&reply_id, legacy_initialize(&params)))
        }
        // No revision defines headers for a notification POST.
        Header::Absent if notification => Prepared::Reply { status: 202, body: None },
        Header::Absent => unsupported(&reply_id, None),
        Header::One(v) => unsupported(&reply_id, Some(v)),
    }
}

fn modern<B: Backend>(
    server: &McpServer<B>,
    view: &HttpView,
    id: Value,
    notification: bool,
    method: &str,
    params: &Value,
    meta_version: Option<&str>,
) -> Prepared {
    // This revision defines no client notification over HTTP, nor headers for one.
    if notification {
        return Prepared::Reply { status: 202, body: None };
    }
    if meta_version != Some(MODERN_VERSION) {
        return mismatch(&id, "the MCP-Protocol-Version header does not match the version in _meta");
    }
    if view.mcp_method != Header::One(method) {
        return mismatch(&id, "the Mcp-Method header is missing or does not match the method");
    }
    if params["_meta"].get(META_CAPABILITIES).is_none() {
        return reply(400, error_response(&id, -32602, "_meta needs io.modelcontextprotocol/clientCapabilities", None));
    }
    match method {
        "server/discover" => reply(200, response(&id, discover())),
        "tools/list" => reply(200, response(&id, cacheable(envelope(Era::Modern, server.tools_list()), TOOLS_TTL_MS))),
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return reply(200, error_response(&id, -32602, "Missing tool name", None));
            };
            let named = match view.mcp_name {
                Header::One(header) => decode_name(header).as_deref() == Some(name),
                _ => false,
            };
            if !named {
                return mismatch(&id, "the Mcp-Name header is missing or does not match the tool name");
            }
            call(id, name, params, Era::Modern)
        }
        _ => reply(404, error_response(&id, -32601, &format!("Method not found: {method}"), None)),
    }
}

fn legacy<B: Backend>(server: &McpServer<B>, id: Value, notification: bool, method: &str, params: &Value) -> Prepared {
    // `notifications/initialized` and `notifications/cancelled` (ignored: nothing to cancel in a
    // stateless server; a disconnect stops the call, #601 rev 8 §4.3).
    if notification {
        return Prepared::Reply { status: 202, body: None };
    }
    match method {
        "initialize" => reply(200, response(&id, legacy_initialize(params))),
        "ping" => reply(200, response(&id, json!({}))),
        "tools/list" => reply(200, response(&id, server.tools_list())),
        "tools/call" => match params["name"].as_str() {
            Some(name) => call(id, name, params, Era::Legacy),
            None => reply(200, error_response(&id, -32602, "Missing tool name", None)),
        },
        _ => reply(200, error_response(&id, -32601, &format!("Method not found: {method}"), None)),
    }
}

/// The legacy handshake: the requested version when served, else the newest legacy one.
fn legacy_initialize(params: &Value) -> Value {
    let requested = params["protocolVersion"].as_str().unwrap_or_default();
    let version = LEGACY_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(LEGACY_VERSIONS[0]);
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": server_info(),
    })
}

fn discover() -> Value {
    let mut versions = vec![MODERN_VERSION];
    versions.extend(LEGACY_VERSIONS);
    cacheable(
        envelope(Era::Modern, json!({ "supportedVersions": versions, "capabilities": { "tools": {} } })),
        DISCOVER_TTL_MS,
    )
}

fn call(id: Value, name: &str, params: &Value, era: Era) -> Prepared {
    Prepared::Call(ToolCall {
        id,
        name: name.to_string(),
        arguments: params.get("arguments").cloned().unwrap_or_else(|| json!({})),
        era,
    })
}

fn server_info() -> Value {
    json!({ "name": SERVER_NAME, "version": SERVER_VERSION })
}

/// A modern result carries `resultType` and the server's identity in `_meta`; a legacy result
/// keeps its legacy shape.
fn envelope(era: Era, mut result: Value) -> Value {
    if era == Era::Modern
        && let Some(object) = result.as_object_mut()
    {
        object.entry("resultType").or_insert(json!("complete"));
        let meta = object.entry("_meta").or_insert(json!({}));
        if let Some(meta) = meta.as_object_mut() {
            meta.insert(META_SERVER.to_string(), server_info());
        }
    }
    result
}

/// Results a client may cache: the same for every caller, but only behind authentication.
fn cacheable(mut result: Value, ttl_ms: u64) -> Value {
    if let Some(object) = result.as_object_mut() {
        object.insert("ttlMs".to_string(), json!(ttl_ms));
        object.insert("cacheScope".to_string(), json!("private"));
    }
    result
}

/// An `Mcp-Name` value: plain, or `=?base64?…?=` when the name is not header-safe.
fn decode_name(header: &str) -> Option<String> {
    match header.strip_prefix("=?base64?").and_then(|rest| rest.strip_suffix("?=")) {
        Some(encoded) => {
            let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).ok()?;
            String::from_utf8(bytes).ok()
        }
        None => Some(header.to_string()),
    }
}

fn reply(status: u16, body: Value) -> Prepared {
    Prepared::Reply { status, body: Some(body) }
}

fn mismatch(id: &Value, what: &str) -> Prepared {
    reply(400, error_response(id, -32020, &format!("Header mismatch: {what}"), None))
}

fn unsupported(id: &Value, requested: Option<&str>) -> Prepared {
    let mut supported = vec![MODERN_VERSION];
    supported.extend(LEGACY_VERSIONS);
    reply(
        400,
        error_response(
            id,
            -32022,
            "Unsupported protocol version",
            Some(json!({ "supported": supported, "requested": requested })),
        ),
    )
}

fn response(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

/// An error response. A request whose id could not be read is answered without one: an MCP id is
/// never `null` (2025-11-25 and 2026-07-28 `JSONRPCErrorResponse`, `id?`).
fn error_response(id: &Value, code: i32, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    let mut response = json!({ "jsonrpc": "2.0", "error": error });
    if !id.is_null() {
        response["id"] = id.clone();
    }
    response
}
