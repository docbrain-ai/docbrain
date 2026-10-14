// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// DocBrain MCP Server — exposes documentation intelligence as MCP tools.
///
/// Communicates via JSON-RPC over stdio (stdin/stdout), following the
/// Model Context Protocol specification. Calls the DocBrain API server
/// for actual processing.
const SERVER_NAME: &str = "docbrain";
/// Longest withdraw reason, in characters; mirrors docbrain-core `fragments::lifecycle::WITHDRAW_REASON_MAX_CHARS` (this crate cannot depend on core).
const WITHDRAW_REASON_MAX_CHARS: usize = 1000;
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// HTTP header name identifying this binary as the caller to DocBrain.
///
/// DocBrain's `/api/v1/ask` handler reads `X-DocBrain-Caller` to set the
/// request's `Surface` (computed server-side).
/// `mcp-host` -> `Surface::McpHost`, which calls live tools by default (R8 /
/// #443): the host's own tools are not the sources DocBrain consults.
///
/// Producer/receiver contract: this value MUST match the literal `"mcp-host"`
/// in `compute_surface()`. Mis-spelling either side silently breaks the
/// surface-detection round-trip (receiver / emitter).
/// What the binary was asked to do, decided from its arguments alone.
///
/// Argument handling must happen BEFORE credential validation: `--version` and
/// `--help` are questions about the binary, not requests to talk to a server,
/// and answering them must not require `DOCBRAIN_API_KEY`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliAction {
    /// Print the version and exit 0.
    Version,
    /// Print usage and exit 0.
    Help,
    /// Run the JSON-RPC stdin/stdout loop (the default, and what an MCP host
    /// invokes).
    Serve,
    /// An argument we do not understand. Reported on stderr, then Serve still
    /// runs — the npm wrapper forwards `process.argv.slice(2)` to this binary
    /// (`npm-mcp/run.js`), so exiting on an unrecognised argument would hard-
    /// break any MCP host that passes one, where the previous code ignored
    /// arguments entirely. A human driving it by hand sees the warning and the
    /// usage text; a host keeps working.
    Unknown(String),
}

/// Decide what to do from the argument list (excluding argv[0]).
pub fn parse_cli(args: &[String]) -> CliAction {
    match args.first().map(String::as_str) {
        None => CliAction::Serve,
        Some("--version") | Some("-V") => CliAction::Version,
        Some("--help") | Some("-h") => CliAction::Help,
        Some(other) => CliAction::Unknown(other.to_string()),
    }
}

/// Usage text for `--help`. Kept next to `parse_cli` so the accepted flags and
/// the documented flags cannot drift apart.
pub fn usage() -> String {
    format!(
        "docbrain-mcp {}\n\
         \n\
         The DocBrain MCP connector. Speaks JSON-RPC over stdin/stdout and is\n\
         normally launched by an MCP host (Claude Code, Cursor, ...), not by hand.\n\
         \n\
         USAGE:\n\
         \x20   docbrain-mcp              run the JSON-RPC loop\n\
         \x20   docbrain-mcp --version    print version\n\
         \x20   docbrain-mcp --help       print this help\n\
         \n\
         ENVIRONMENT:\n\
         \x20   DOCBRAIN_API_KEY      required to serve; create with\n\
         \x20                         `docbrain token create --name \"MCP Key\" --role viewer`\n\
         \x20   DOCBRAIN_SERVER_URL   defaults to http://localhost:3000\n",
        env!("CARGO_PKG_VERSION")
    )
}

pub const DOCBRAIN_CALLER_HEADER: &str = "X-DocBrain-Caller";
pub const DOCBRAIN_CALLER_VALUE: &str = "mcp-host";

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

pub struct McpServer {
    pub server_url: String,
    pub api_key: Option<String>,
    pub client: reqwest::Client,
    /// For `/api/v1/ask` only: no client timeout, as the CLI's ask client. An ask
    /// that calls live tools can outlast `client`'s 120 s, and the server's own turn
    /// clock is what bounds it (#443).
    ask_client: reqwest::Client,
    /// Caller role string ("viewer"/"editor"/"analyst"/"admin"), resolved once
    /// from `/api/v1/auth/me` in `validate_connection`. `None` until validated.
    /// Used to fail-fast write-tool calls from a viewer-scoped key with a clear
    /// message; the server API endpoints remain the authoritative gate.
    role: std::sync::OnceLock<String>,
}

/// Numeric rank of a DocBrain role string for write-gating. Unknown → lowest.
/// Local to this crate so we don't pull in docbrain-core just for the enum.
fn role_rank(role: &str) -> u8 {
    match role {
        "admin" => 3,
        "analyst" => 2,
        "editor" => 1,
        _ => 0, // viewer / unknown
    }
}

/// The claim-verification verdict, as the lines a host reads around an answer.
///
/// Returns `(before, after)`.
///
/// Wording comes from the server's own `notes` (produced by
/// `claim_verifier::render_note`) and is never re-derived here, so this shim,
/// the web banner and the CLI cannot drift apart on what a verdict says.
///
/// A FAILED verdict goes BEFORE the answer. That mirrors the web banner, which
/// sits above the confidence line deliberately — burying a failed check under
/// the answer inverts which of the two signals is stronger. It also matters
/// more here than on the web: an MCP host owns the pixels, so whatever it reads
/// first is what it is least able to paraphrase away.
///
/// A PASS is one quiet line after, worded exactly as the web words it, so that
/// silence is never the only signal — absence of any line would otherwise read
/// as "this feature is switched off".
///
/// `nothing_checkable` and an absent `verification` produce NEITHER. The first
/// is the outcome for every answer on a deployment whose sources record no file
/// listings, where a line on each one is indistinguishable from a broken
/// feature. The second means the pass never ran, and reporting a verdict that
/// was never computed is the exact failure `Answer::verification` being an
/// `Option` exists to prevent.
fn verification_lines(response: &Value) -> (String, String) {
    let Some(v) = response.get("verification") else {
        return (String::new(), String::new());
    };
    match v["status"].as_str() {
        Some("failed") => {
            let notes: Vec<&str> = v["notes"]
                .as_array()
                .map(|a| a.iter().filter_map(|n| n.as_str()).collect())
                .unwrap_or_default();
            if notes.is_empty() {
                // Failed with nothing renderable: say nothing rather than
                // assert a failure the reader cannot act on.
                return (String::new(), String::new());
            }
            (format!("{}\n\n", notes.join("\n")), String::new())
        }
        Some("passed") => {
            let n = v["checked"].as_u64().unwrap_or(0);
            if n == 0 {
                return (String::new(), String::new());
            }
            let noun = if n == 1 { "path" } else { "paths" };
            (
                String::new(),
                format!("\n\n{n} referenced {noun} checked against the source."),
            )
        }
        _ => (String::new(), String::new()),
    }
}

/// The answer's own page, when the server offered one as an absolute link: one visible line the
/// reader can open later. A relative path alone is not shown; an editor has nothing to resolve it against.
fn answer_page_line(response: &Value) -> Option<String> {
    let url = response["answer_url"].as_str()?.trim();
    (url.starts_with("https://") || url.starts_with("http://"))
        .then(|| format!("\n\nAnswer page: {url}"))
}

/// Everything the server wants shown before the answer, in the server's order.
///
/// Reads `blocks` and never matches on `kind`: a kind this build has never heard
/// of still reaches the host, because the server decides what the reader needs.
/// Falls back to `stale_claims` when `blocks` is absent, so an older server
/// keeps working.
fn leading_lines(response: &Value) -> String {
    if let Some(blocks) = response.get("blocks").and_then(|v| v.as_array()) {
        let lines: Vec<&str> = blocks
            .iter()
            .filter(|b| b.get("kind").and_then(|k| k.as_str()) != Some("answer"))
            .filter_map(|b| b.get("text")?.as_str())
            .filter(|s| !s.is_empty())
            .collect();
        if lines.is_empty() {
            return String::new();
        }
        return format!("{}\n\n", lines.join("\n"));
    }
    let Some(claims) = response.get("stale_claims").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let notes: Vec<&str> = claims.iter().filter_map(|c| c.get("note")?.as_str()).collect();
    if notes.is_empty() {
        return String::new();
    }
    format!("{}\n\n", notes.join("\n"))
}

impl McpServer {
    // `Default` would silently read process env (`DOCBRAIN_SERVER_URL`/
    // `DOCBRAIN_API_KEY`), which is surprising for a Default impl. Callers
    // should pick `new()` (env-driven) or `from_parts()` (explicit).
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let server_url = std::env::var("DOCBRAIN_SERVER_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());
        let api_key = std::env::var("DOCBRAIN_API_KEY").ok();
        Self {
            server_url,
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("Failed to build HTTP client"),
            ask_client: reqwest::Client::new(),
            role: std::sync::OnceLock::new(),
        }
    }

    /// Test-friendly constructor that avoids touching process env vars.
    ///
    /// Production code (`fn main`) uses `McpServer::new()` which reads
    /// `DOCBRAIN_SERVER_URL` / `DOCBRAIN_API_KEY` from the environment.
    /// Integration tests must not mutate process env in parallel runs, so
    /// they construct the server with explicit values instead.
    pub fn from_parts(server_url: String, api_key: Option<String>) -> Self {
        Self {
            server_url,
            api_key,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("Failed to build HTTP client"),
            ask_client: reqwest::Client::new(),
            role: std::sync::OnceLock::new(),
        }
    }

    /// Validate API key and server connectivity at startup.
    /// Returns the display name of the authenticated identity on success.
    pub async fn validate_connection(&self) -> Result<String, String> {
        let url = format!("{}/api/v1/auth/me", self.server_url);
        // Caller header is set BEFORE the auth header so anonymous calls
        // (no DOCBRAIN_API_KEY) still identify their surface even when the
        // downstream returns 401. See DOCBRAIN_CALLER_HEADER docstring.
        let mut request = self
            .client
            .get(&url)
            .header(DOCBRAIN_CALLER_HEADER, DOCBRAIN_CALLER_VALUE);
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        match request.send().await {
            Ok(resp) if resp.status().is_success() => {
                let identity: Value = resp
                    .json()
                    .await
                    .map_err(|e| format!("Invalid /auth/me response: {}", e))?;
                let name = identity["name"]
                    .as_str()
                    .or_else(|| identity["email"].as_str())
                    .unwrap_or("(unknown)")
                    .to_string();
                let role = identity["role"].as_str().unwrap_or("unknown");
                // Cache the role for write-tool gating. set() is
                // idempotent for our single-validate flow; ignore already-set.
                let _ = self.role.set(role.to_string());
                Ok(format!("{} ({})", name, role))
            }
            Ok(resp) if resp.status() == 401 => {
                Err("Invalid or expired DOCBRAIN_API_KEY (401 Unauthorized). \
                     Run: docbrain token create --name \"MCP Key\" --role viewer"
                    .to_string())
            }
            Ok(resp) => Err(format!(
                "Server returned unexpected status: {}",
                resp.status()
            )),
            Err(e) if e.is_connect() => Err(format!(
                "Cannot connect to DocBrain server at {}. \
                     Check DOCBRAIN_SERVER_URL.",
                self.server_url
            )),
            Err(e) => Err(format!("Connection error: {}", e)),
        }
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let id = req.id.clone().unwrap_or(Value::Null);

        let result = match req.method.as_str() {
            "initialize" => Ok(self.handle_initialize()),
            "initialized" | "notifications/initialized" => {
                return JsonRpcResponse {
                    jsonrpc: "2.0".into(),
                    id,
                    result: None,
                    error: None,
                };
            }
            "tools/list" => Ok(self.handle_tools_list()),
            "tools/call" => self.handle_tools_call(&req.params).await,
            "ping" => Ok(json!({})),
            _ => Err(JsonRpcError {
                code: -32601,
                message: format!("Method not found: {}", req.method),
            }),
        };

        match result {
            Ok(value) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id,
                result: Some(value),
                error: None,
            },
            Err(error) => JsonRpcResponse {
                jsonrpc: "2.0".into(),
                id,
                result: None,
                error: Some(error),
            },
        }
    }

    fn handle_initialize(&self) -> Value {
        json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            }
        })
    }

    fn handle_tools_list(&self) -> Value {
        json!({
            "tools": [
                {
                    "name": "docbrain_ask",
                    "description": "Ask a question about your organization's internal documentation. Uses hybrid semantic + keyword search across Confluence, runbooks, and captured Slack/GitHub threads. Preserves specific service names, team names, and tool names for precise retrieval. Corrects false premises (e.g. if you assume every engineer can deploy to production, it will correct you). Returns a direct, human-like answer — never says 'according to the documentation'. Cites up to 5 real source documents with URLs. For questions outside the knowledge base (HR policies, general engineering topics not documented internally), clearly says so rather than guessing. Uses session memory to resolve follow-up questions in context.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "The question to ask. Use specific service/team/tool names for best results (e.g. 'payments service logs', 'production database access for engineers', 'nightly batch job deployment')."
                            },
                            "session_id": {
                                "type": "string",
                                "description": SESSION_ID_DESCRIPTION
                            }
                        },
                        "required": ["question"]
                    }
                },
                {
                    "name": "docbrain_incident",
                    "description": "Incident-mode search: prioritizes runbooks, past incident resolutions, on-call procedures, and troubleshooting guides. Boosts results from incident postmortems and operational docs. Use during active incidents or when debugging production issues — gives faster, more targeted answers than docbrain_ask for operational questions.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "description": {
                                "type": "string",
                                "description": "Description of the incident, error message, or system being debugged. Be specific — include service names, error codes, and symptoms."
                            }
                        },
                        "required": ["description"]
                    }
                },
                {
                    "name": "docbrain_freshness",
                    "description": "Get a freshness report for documentation. Shows which docs are fresh, stale, or outdated.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "space": {
                                "type": "string",
                                "description": "Confluence space key to filter by (optional, omit for all spaces)"
                            }
                        }
                    }
                },
                {
                    "name": "docbrain_autopilot_gaps",
                    "description": "List documentation gaps detected by Autopilot. Shows clusters of unanswered or poorly-answered questions, ranked by severity. Use this to understand what documentation is missing.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "limit": {
                                "type": "number",
                                "description": "Maximum number of gap clusters to return (default: 10)"
                            }
                        }
                    }
                },
                {
                    "name": "docbrain_autopilot_generate",
                    "description": "Generate a documentation draft for a specific gap cluster. Autopilot uses existing docs as context to draft the missing content (runbook, FAQ, guide, etc). Returns the draft content for review.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "cluster_id": {
                                "type": "string",
                                "description": "UUID of the gap cluster to generate a draft for. Get IDs from docbrain_autopilot_gaps."
                            }
                        },
                        "required": ["cluster_id"]
                    }
                },
                {
                    "name": "docbrain_autopilot_summary",
                    "description": "Get a summary of Autopilot status: total gaps, open gaps, critical gaps, drafts generated, and drafts published.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {}
                    }
                },
                {
                    "name": "docbrain_annotate",
                    "description": with_secret_guardrail("Capture knowledge linked to a specific code location. Creates a knowledge fragment tied to a file and line range. Use this when you encounter important decisions, caveats, or context in code that should be preserved as organizational knowledge."),
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Path to the source file (relative to repo root)"
                            },
                            "line_range": {
                                "type": "string",
                                "description": "Line range in the file, e.g. '42-58'. Optional — omit for file-level annotations."
                            },
                            "annotation": {
                                "type": "string",
                                "description": with_secret_guardrail("The knowledge to capture — a decision, fact, caveat, procedure, or context (max 10000 chars).")
                            },
                            "fragment_type": {
                                "type": "string",
                                "enum": ["decision", "fact", "caveat", "procedure", "context"],
                                "description": "Type of knowledge. decision = why something was chosen, fact = how something works, caveat = gotcha or limitation, procedure = step-by-step process, context = background info. Defaults to 'context'."
                            },
                            "code_snippet": {
                                "type": "string",
                                "description": "The actual code at the annotated location. Used to compute a hash for drift detection — if the code changes later, the annotation is flagged as potentially stale."
                            },
                            "space": {
                                "type": "string",
                                "description": "Documentation space to associate with (e.g. team or project name). Optional."
                            },
                            "gap_id": {
                                "type": "string",
                                "description": "Optional: the id of an open question (a gap) this capture answers; it closes that question"
                            },
                            "premises": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "premise_type": { "type": "string" },
                                        "expression": { "type": "string" }
                                    },
                                    "required": ["premise_type", "expression"]
                                },
                                "description": "Optional machine-checkable premises this annotation rests on; v1 checks type 'path'"
                            }
                        },
                        "required": ["file_path", "annotation"]
                    }
                },
                {
                    "name": "docbrain_suggest_capture",
                    "description": "Check if there are unanswered questions or documentation gaps related to a file or function. Returns suggestions for what knowledge is missing and should be captured. Use this proactively when working in complex or undocumented code areas.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "file_path": {
                                "type": "string",
                                "description": "Path to the source file to check for knowledge gaps"
                            },
                            "function_name": {
                                "type": "string",
                                "description": "Specific function or method name to check. Optional — omit to check the entire file."
                            }
                        },
                        "required": ["file_path"]
                    }
                },
                {
                    "name": "docbrain_context",
                    "description": "Before changing files, ask what this organization already knows about them. Returns decisions, caveats and constraints captured against those exact paths, with a warning first if any of that knowledge has since gone stale. Use it when you are about to edit, refactor or delete code — not as a search tool. Paths must be bare, with no line suffix: \"src/auth/session.rs\" is correct, \"src/auth/session.rs:42\" is wrong and will be rejected.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "file_paths": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "Repo-relative paths about to change, e.g. [\"src/auth/session.rs\"]. No line suffix: \"src/auth/session.rs:42\" is wrong and will be rejected. Max 100."
                            }
                        },
                        "required": ["file_paths"]
                    }
                },
                {
                    "name": "docbrain_commit_capture",
                    "description": with_secret_guardrail("Capture knowledge from a commit — why was this change made? Creates a knowledge fragment from the commit intent, grounding it in the diff and commit message. Use this at commit time to preserve the reasoning behind changes before it's forgotten."),
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "diff_summary": {
                                "type": "string",
                                "description": with_secret_guardrail("Summary of what changed in the diff (max 5000 chars). Can be a condensed diff or a human-readable summary of files/functions changed.")
                            },
                            "commit_message": {
                                "type": "string",
                                "description": with_secret_guardrail("The commit message (max 1000 chars).")
                            },
                            "intent": {
                                "type": "string",
                                "description": with_secret_guardrail("Why was this change made? The reasoning, trade-offs, alternatives considered, or decisions that led to this commit (max 10000 chars).")
                            },
                            "file_paths": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "List of files changed in the commit. Each one becomes a premise this capture is checked against, so the capture is flagged if that file is later moved or deleted. Send them — a commit capture with no files can never be checked against the code again."
                            },
                            "space": {
                                "type": "string",
                                "description": "Documentation space to associate with. Optional."
                            },
                            "gap_id": {
                                "type": "string",
                                "description": "Optional: the id of an open question (a gap) this capture answers; it closes that question"
                            },
                            "premises": {
                                "type": "array",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "premise_type": { "type": "string" },
                                        "expression": { "type": "string" }
                                    },
                                    "required": ["premise_type", "expression"]
                                },
                                "description": "Optional machine-checkable premises this reasoning rests on, beyond the changed files; v1 checks type 'path'"
                            }
                        },
                        "required": ["intent"]
                    }
                },
                {
                    "name": "docbrain_revise",
                    "description": with_secret_guardrail("Edit or withdraw a capture you recorded. Use the id from docbrain_context or from the capture's report. Edit replaces its text and/or its premise set (the same premises as docbrain_annotate) and re-indexes it; withdraw takes it out of answers at once and needs a reason, which is shown with it. Only its author can edit; DocBrain-written summaries can only be withdrawn. A capture already used in an answer cannot be deleted, only withdrawn."),
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string", "description": "The capture's id" },
                            "text": { "type": "string", "description": with_secret_guardrail("The full replacement text (max 10000 characters).") },
                            "premises": {
                                "type": "array",
                                "description": "The full replacement premise set",
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "premise_type": { "type": "string" },
                                        "expression": { "type": "string" }
                                    },
                                    "required": ["premise_type", "expression"]
                                }
                            },
                            "withdraw_reason": { "type": "string", "description": with_secret_guardrail("Withdraw instead of editing; why (max 1000 characters, shown with it).") }
                        },
                        "required": ["id"]
                    }
                },
                {
                    "name": "docbrain_feedback",
                    "description": "Submit feedback on a DocBrain answer to improve future quality. Use after docbrain_ask or docbrain_incident — pass the episode_id from that response. Negative feedback (thumbs_up=false) triggers Autopilot gap analysis: if a question is repeatedly unanswered, Autopilot drafts the missing documentation automatically.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "episode_id": {
                                "type": "string",
                                "description": "UUID of the episode to rate. Returned in the docbrain_ask or docbrain_incident response."
                            },
                            "thumbs_up": {
                                "type": "boolean",
                                "description": "true = answer was correct and helpful. false = answer was wrong, incomplete, or unhelpful."
                            },
                            "reason": {
                                "type": "string",
                                "description": "Why the answer was unhelpful (only when thumbs_up=false). One of: incorrect, incomplete, outdated, not_relevant, doc_exists."
                            },
                            "note": {
                                "type": "string",
                                "description": "Optional free-text correction or comment (max 280 chars)."
                            }
                        },
                        "required": ["episode_id", "thumbs_up"]
                    }
                }
            ]
        })
    }

    async fn handle_tools_call(&self, params: &Value) -> Result<Value, JsonRpcError> {
        let tool_name = params["name"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing tool name".into(),
        })?;
        let arguments = &params["arguments"];

        // Write-capable IDE tools require at least Editor. A viewer-scoped MCP
        // key must not commit captures/annotations or trigger autopilot
        // generation. The server-side API endpoints are the authoritative gate;
        // this returns a fast, actionable error before the round-trip. An
        // unvalidated/unknown role is denied (fail-closed).
        const WRITE_TOOLS: &[&str] = &[
            "docbrain_commit_capture",
            "docbrain_annotate",
            "docbrain_autopilot_generate",
            "docbrain_revise",
        ];
        if WRITE_TOOLS.contains(&tool_name) {
            let role = self.role.get().map(String::as_str);
            if role.map(role_rank).unwrap_or(0) < role_rank("editor") {
                return Err(JsonRpcError {
                    code: -32004,
                    message: format!(
                        "Tool '{}' requires an editor-or-higher API key; this key is {}. \
                         Create one with: docbrain token create --name \"MCP Key\" --role editor",
                        tool_name,
                        role.unwrap_or("unverified")
                    ),
                });
            }
        }

        match tool_name {
            "docbrain_ask" => self.tool_ask(arguments).await,
            "docbrain_incident" => self.tool_incident(arguments).await,
            "docbrain_freshness" => self.tool_freshness(arguments).await,
            "docbrain_autopilot_gaps" => self.tool_autopilot_gaps(arguments).await,
            "docbrain_autopilot_generate" => self.tool_autopilot_generate(arguments).await,
            "docbrain_autopilot_summary" => self.tool_autopilot_summary(arguments).await,
            "docbrain_annotate" => self.tool_annotate(arguments).await,
            "docbrain_suggest_capture" => self.tool_suggest_capture(arguments).await,
            "docbrain_context" => self.tool_context(arguments).await,
            "docbrain_commit_capture" => self.tool_commit_capture(arguments).await,
            "docbrain_feedback" => self.tool_feedback(arguments).await,
            "docbrain_revise" => self.tool_revise(arguments).await,
            _ => Err(JsonRpcError {
                code: -32602,
                message: format!("Unknown tool: {}", tool_name),
            }),
        }
    }

    async fn tool_ask(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let question = args["question"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'question' parameter".into(),
        })?;
        let session_id = args["session_id"].as_str();

        let mut body = json!({ "question": question });
        if let Some(sid) = session_id {
            body["session_id"] = json!(sid);
        }
        let response = match self.post(&self.ask_client, "/api/v1/ask", &body).await {
            Ok(v) => v,
            Err(f) if session_id.is_some() && f.names_a_dead_session() => {
                return Err(JsonRpcError {
                    code: -32000,
                    message: START_NEW_CONVERSATION.into(),
                });
            }
            Err(f) => return Err(f.into()),
        };

        let answer = response["answer"].as_str().unwrap_or("No answer available");
        let sources: Vec<String> = response["sources"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let title = s["title"].as_str().unwrap_or("Source");
                        let url = s["source_url"].as_str().unwrap_or("");
                        if url.is_empty() {
                            None
                        } else {
                            Some(format!("- [{}]({})", title, url))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let (verdict_before, verdict_after) = verification_lines(&response);
        let leading = leading_lines(&response);
        let mut text = format!("{leading}{verdict_before}{answer}");
        if !sources.is_empty() {
            text.push_str("\n\nSources:\n");
            text.push_str(&sources.join("\n"));
        }
        if let Some(line) = answer_page_line(&response) {
            text.push_str(&line);
        }
        if let Some(episode_id) = response["episode_id"].as_str() {
            text.push_str(&format!(
                "\n\n<!-- episode_id: {} — pass to docbrain_feedback to rate this answer -->",
                episode_id
            ));
        }
        text.push_str(&verdict_after);
        if let Some(sid) = response["session_id"].as_str() {
            text.push_str(&format!(
                "\n\n<!-- session_id: {sid} — pass as session_id to docbrain_ask to ask a follow-up in this conversation -->"
            ));
        }

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    async fn tool_incident(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let description = args["description"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'description' parameter".into(),
        })?;

        let body = json!({ "description": description });
        let response = self.api_call("/api/v1/incident", &body).await?;

        let answer = response["answer"]
            .as_str()
            .unwrap_or("No incident response available");
        let sources: Vec<String> = response["sources"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| {
                        let title = s["title"].as_str().unwrap_or("Source");
                        let url = s["source_url"].as_str().unwrap_or("");
                        if url.is_empty() {
                            None
                        } else {
                            Some(format!("- [{}]({})", title, url))
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        let (verdict_before, verdict_after) = verification_lines(&response);
        let leading = leading_lines(&response);
        // Above the header, not below it: mid-incident, a reader acts on the
        // first line and this one says not to trust part of what follows.
        let mut text = format!("{leading}{verdict_before}**INCIDENT RESPONSE**\n\n{}", answer);
        if !sources.is_empty() {
            text.push_str("\n\nRunbooks & Sources:\n");
            text.push_str(&sources.join("\n"));
        }
        if let Some(line) = answer_page_line(&response) {
            text.push_str(&line);
        }
        if let Some(episode_id) = response["episode_id"].as_str() {
            text.push_str(&format!(
                "\n\n<!-- episode_id: {} — pass to docbrain_feedback to rate this response -->",
                episode_id
            ));
        }
        text.push_str(&verdict_after);

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    async fn tool_freshness(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let space = args["space"].as_str();

        let mut url = format!("{}/api/v1/freshness", self.server_url);
        if let Some(s) = space {
            let encoded: String = s
                .bytes()
                .map(|b| {
                    if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' {
                        format!("{}", b as char)
                    } else {
                        format!("%{:02X}", b)
                    }
                })
                .collect();
            url = format!("{}?space={}", url, encoded);
        }

        let mut request = self.client.get(&url);
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;

        let data: Value = response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;

        let summary = &data["summary"];
        let scope = data["space"].as_str().unwrap_or("All spaces");

        let mut text = format!(
            "Freshness Report: {}\nTotal: {} | Avg: {:.0} | Fresh: {} | Review: {} | Stale: {} | Outdated: {}",
            scope,
            summary["total_docs"],
            summary["avg_score"].as_f64().unwrap_or(0.0),
            summary["fresh"],
            summary["review"],
            summary["stale"],
            summary["outdated"]
        );

        if let Some(docs) = data["documents"].as_array() {
            let stalest: Vec<String> = docs
                .iter()
                .take(10)
                .filter_map(|d| {
                    let title = d["title"].as_str()?;
                    let score = d["total_score"].as_f64()?;
                    let status = d["status"].as_str().unwrap_or("unknown");
                    let url = d["source_url"].as_str().unwrap_or("");
                    Some(format!(
                        "- {} (score: {:.0}, {}) {}",
                        title, score, status, url
                    ))
                })
                .collect();

            if !stalest.is_empty() {
                text.push_str("\n\nStalest docs:\n");
                text.push_str(&stalest.join("\n"));
            }
        }

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    async fn tool_autopilot_gaps(&self, args: &Value) -> Result<Value, JsonRpcError> {
        // Clamp to 1..=200 (default 10): a negative/zero limit produces a
        // nonsensical query and an unbounded value would pull the whole gap
        // table in one call from a hallucinated arg.
        let limit = args["limit"].as_i64().unwrap_or(10).clamp(1, 200);

        let mut request = self.client.get(format!(
            "{}/api/v1/autopilot/gaps?limit={}",
            self.server_url, limit
        ));
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;

        let gaps: Vec<Value> = response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;

        if gaps.is_empty() {
            return Ok(json!({
                "content": [{
                    "type": "text",
                    "text": "No documentation gaps detected yet. Autopilot needs usage data — ask questions and provide feedback to populate gap analysis."
                }]
            }));
        }

        let mut text = format!("Documentation Gaps ({} clusters)\n\n", gaps.len());

        for (i, gap) in gaps.iter().enumerate() {
            let label = gap["label"].as_str().unwrap_or("Unknown");
            let severity = gap["severity"].as_str().unwrap_or("unknown");
            let count = gap["query_count"].as_i64().unwrap_or(0);
            let confidence = gap["avg_confidence"].as_f64().unwrap_or(0.0);
            let description = gap["description"].as_str().unwrap_or("");
            let id = gap["id"].as_str().unwrap_or("");

            text.push_str(&format!(
                "{}. [{}] {} ({} queries, confidence: {:.2})\n",
                i + 1,
                severity.to_uppercase(),
                label,
                count,
                confidence
            ));
            text.push_str(&format!("   {}\n", description));

            if let Some(samples) = gap["sample_queries"].as_array() {
                let sample_strs: Vec<&str> =
                    samples.iter().take(3).filter_map(|s| s.as_str()).collect();
                if !sample_strs.is_empty() {
                    text.push_str(&format!("   Sample queries: {}\n", sample_strs.join("; ")));
                }
            }
            text.push_str(&format!("   Cluster ID: {}\n\n", id));
        }

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    async fn tool_autopilot_generate(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let cluster_id = args["cluster_id"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'cluster_id' parameter. Get IDs from docbrain_autopilot_gaps.".into(),
        })?;

        let body = json!({});
        let url = format!(
            "{}/api/v1/autopilot/generate/{}",
            self.server_url, cluster_id
        );

        let mut request = self.client.post(&url).json(&body);
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(JsonRpcError {
                code: -32000,
                message: format!("API error ({}): {}", status, body),
            });
        }

        let result: Value = response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;

        // Now fetch the full draft to get the content
        let draft_id = result["draft_id"].as_str().unwrap_or("");
        let title = result["title"].as_str().unwrap_or("Untitled");
        let content_type = result["content_type"].as_str().unwrap_or("unknown");
        let quality = result["quality_score"].as_f64().unwrap_or(0.0);

        let mut text = format!(
            "Draft Generated: {} (type: {}, quality: {:.2})\nDraft ID: {}\n\n",
            title, content_type, quality, draft_id
        );

        // Fetch the draft content
        if !draft_id.is_empty() {
            let draft_url = format!("{}/api/v1/autopilot/drafts/{}", self.server_url, draft_id);
            let mut draft_request = self.client.get(&draft_url);
            if let Some(ref key) = self.api_key {
                draft_request = draft_request.header("Authorization", format!("Bearer {}", key));
            }
            if let Ok(draft_resp) = draft_request.send().await
                && let Ok(draft) = draft_resp.json::<Value>().await
                && let Some(content) = draft["content"].as_str()
            {
                text.push_str("--- Draft Content ---\n\n");
                text.push_str(content);
            }
        }

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    async fn tool_autopilot_summary(&self, _args: &Value) -> Result<Value, JsonRpcError> {
        let mut request = self
            .client
            .get(format!("{}/api/v1/autopilot/summary", self.server_url));
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;

        let data: Value = response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;

        let text = format!(
            "Autopilot Summary\n\
             Total gap clusters: {}\n\
             Open gaps: {}\n\
             Critical gaps: {}\n\
             Drafts generated: {}\n\
             Drafts published: {}\n\
             Last analysis: {}",
            data["total_gaps"],
            data["open_gaps"],
            data["critical_gaps"],
            data["drafts_generated"],
            data["drafts_published"],
            data["last_analysis_at"].as_str().unwrap_or("never")
        );

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    /// `docbrain_annotate` — capture knowledge linked to a code location.
    ///
    /// Creates a knowledge fragment via `POST /api/v1/fragments` with:
    /// - `source_type: ide_annotation`
    /// - `code_location: file_path:line_range` (or just file_path)
    /// - `code_hash: SHA-256(code_snippet)` for drift detection
    async fn tool_annotate(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let gap_id = gap_cluster_id(args)?;
        let file_path = args["file_path"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'file_path' parameter".into(),
        })?;
        let annotation = args["annotation"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'annotation' parameter".into(),
        })?;

        // Input validation
        if file_path.trim().is_empty() {
            return Err(JsonRpcError {
                code: -32602,
                message: "file_path must not be empty".into(),
            });
        }
        if annotation.trim().is_empty() {
            return Err(JsonRpcError {
                code: -32602,
                message: "annotation must not be empty".into(),
            });
        }
        if annotation.chars().count() > 10_000 {
            return Err(JsonRpcError {
                code: -32602,
                message: "annotation exceeds 10000 character limit".into(),
            });
        }
        if file_path.chars().count() > 500 {
            return Err(JsonRpcError {
                code: -32602,
                message: "file_path exceeds 500 character limit".into(),
            });
        }

        let line_range = args["line_range"].as_str();
        let fragment_type = args["fragment_type"].as_str().unwrap_or("context");
        let space = args["space"].as_str();

        // Validate fragment_type
        let valid_types = ["decision", "fact", "caveat", "procedure", "context"];
        if !valid_types.contains(&fragment_type) {
            return Err(JsonRpcError {
                code: -32602,
                message: format!(
                    "Invalid fragment_type '{}'. Must be one of: {}",
                    fragment_type,
                    valid_types.join(", ")
                ),
            });
        }

        // Build code_location: "path/to/file.rs:42-58" or "path/to/file.rs"
        let code_location = match line_range {
            Some(lr) => format!("{}:{}", file_path, lr),
            None => file_path.to_string(),
        };

        // Compute code_hash from snippet for drift detection
        let code_hash = args["code_snippet"].as_str().map(|snippet| {
            let mut hasher = Sha256::new();
            hasher.update(snippet.as_bytes());
            hex::encode(hasher.finalize())
        });

        // Build source_id for idempotency: include code_hash so changed code → new fragment
        let source_id = match (&code_hash, line_range) {
            (Some(hash), Some(lr)) => format!("ide:{}:{}:{}", file_path, lr, &hash[..16]),
            (Some(hash), None) => format!("ide:{}:{}", file_path, &hash[..16]),
            (None, Some(lr)) => format!("ide:{}:{}", file_path, lr),
            (None, None) => format!("ide:{}", file_path),
        };

        // Build summary from first line/sentence of annotation (UTF-8 safe truncation)
        let summary = annotation.lines().next().unwrap_or(annotation);
        let summary: String = summary.chars().take(200).collect();

        let mut body = json!({
            "fragment_type": fragment_type,
            "summary": summary,
            "content": annotation,
            "source_type": "ide_annotation",
            "source_id": source_id,
            // No confidence field: routing is derived server-side from the
            // capture's anchors (code_location + premises), never from a value
            // the client asserts about its own trustworthiness. Sending one
            // here would read like it controlled whether a human reviews this
            // capture, which it no longer does. Pass `premises` to make the
            // capture falsifiable — that is what earns unreviewed indexing.
            "code_location": code_location,
        });

        if let Some(hash) = &code_hash {
            body["code_hash"] = json!(hash);
        }
        if let Some(s) = space {
            body["space"] = json!(s);
        }
        if let Some(premises) = args["premises"].as_array() {
            body["premises"] = json!(premises);
        }
        if let Some(gap) = &gap_id {
            body["gap_cluster_id"] = json!(gap);
        }

        match self.create_fragment(&body).await? {
            Ok(result) => {
                let drift_note = if code_hash.is_some() {
                    " Code hash saved — annotation will be flagged if the code changes."
                } else {
                    " Tip: pass code_snippet to enable drift detection."
                };

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "Knowledge captured: {} annotation at {}\n{}\n{}",
                            fragment_type, code_location, capture_report(&result), drift_note
                        )
                    }]
                }))
            }
            Err(msg) => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": msg
                }]
            })),
        }
    }

    /// `docbrain_suggest_capture` — surface unanswered questions about a code area.
    ///
    /// Queries autopilot gaps and filters for relevance to the given file/function.
    async fn tool_suggest_capture(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let file_path = args["file_path"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'file_path' parameter".into(),
        })?;
        let function_name = args["function_name"].as_str();

        // Fetch gap clusters from autopilot — get more than needed, filter client-side
        let mut request = self.client.get(format!(
            "{}/api/v1/autopilot/gaps?limit=50",
            self.server_url
        ));
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(JsonRpcError {
                code: -32000,
                message: format!("API error ({}): {}", status, body),
            });
        }

        let gaps: Vec<Value> = response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;

        // Extract search terms from file path and function name
        // Support both Unix and Windows path separators
        let path_parts: Vec<&str> = file_path.split(['/', '\\']).collect();
        let file_name = path_parts.last().copied().unwrap_or(file_path);
        let stem = file_name.split('.').next().unwrap_or(file_name);

        let mut search_terms: Vec<String> = vec![stem.to_lowercase()];
        // Add parent directory as a term (often maps to a module/feature)
        if path_parts.len() >= 2 {
            search_terms.push(path_parts[path_parts.len() - 2].to_lowercase());
        }
        if let Some(func) = function_name {
            // Split camelCase/snake_case into words, preserving uppercase letters
            let mut words = Vec::new();
            let mut current = String::new();
            for c in func.chars() {
                if c == '_' {
                    if current.len() > 2 {
                        words.push(current.to_lowercase());
                    }
                    current.clear();
                } else if c.is_uppercase() && !current.is_empty() {
                    if current.len() > 2 {
                        words.push(current.to_lowercase());
                    }
                    current.clear();
                    current.push(c);
                } else {
                    current.push(c);
                }
            }
            if current.len() > 2 {
                words.push(current.to_lowercase());
            }
            search_terms.extend(words);
            search_terms.push(func.to_lowercase());
        }

        // Filter gaps by relevance: label, description, or sample queries mention search terms
        let relevant: Vec<&Value> = gaps
            .iter()
            .filter(|gap| {
                let label = gap["label"].as_str().unwrap_or("").to_lowercase();
                let desc = gap["description"].as_str().unwrap_or("").to_lowercase();
                let samples: String = gap["sample_queries"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default()
                    .to_lowercase();

                let haystack = format!("{} {} {}", label, desc, samples);
                search_terms
                    .iter()
                    .any(|term| haystack.contains(term.as_str()))
            })
            .take(5)
            .collect();

        if relevant.is_empty() {
            let context = match function_name {
                Some(f) => format!("{}::{}", file_path, f),
                None => file_path.to_string(),
            };
            return Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!(
                        "No documentation gaps found related to {}.\n\n\
                         This area appears well-documented, or no unanswered questions have been asked about it yet.\n\
                         Tip: use docbrain_annotate to proactively capture knowledge about complex code before it's needed.",
                        context
                    )
                }]
            }));
        }

        let context = match function_name {
            Some(f) => format!("{}::{}", file_path, f),
            None => file_path.to_string(),
        };
        let mut text = format!("Knowledge gaps related to {}:\n\n", context);

        for (i, gap) in relevant.iter().enumerate() {
            let label = gap["label"].as_str().unwrap_or("Unknown");
            let severity = gap["severity"].as_str().unwrap_or("unknown");
            let desc = gap["description"].as_str().unwrap_or("");
            let id = gap["id"].as_str().unwrap_or("");

            text.push_str(&format!(
                "{}. [{}] {}\n   {}\n",
                i + 1,
                severity.to_uppercase(),
                label,
                desc
            ));

            if let Some(samples) = gap["sample_queries"].as_array() {
                let sample_strs: Vec<&str> =
                    samples.iter().take(2).filter_map(|s| s.as_str()).collect();
                if !sample_strs.is_empty() {
                    text.push_str(&format!("   Unanswered: {}\n", sample_strs.join("; ")));
                }
            }
            text.push_str(&format!("   Cluster ID: {}\n\n", id));
        }

        text.push_str("Tip: use docbrain_annotate to capture knowledge that answers these gaps, or docbrain_autopilot_generate to auto-draft documentation.");

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    /// `docbrain_context` — what the organisation knows about these paths.
    ///
    /// Renders the server's blocks in the server's order and never filters by
    /// `kind`, for the same reason `leading_lines` does not: the server decides
    /// what the reader needs, and a client that drops what it does not
    /// recognise loses signal the day a kind is added.
    ///
    /// When nothing is captured it says so, naming the paths it checked. An
    /// empty string would let the agent conclude there is no organisational
    /// opinion, which is indistinguishable from not having looked.
    async fn tool_context(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let paths: Vec<String> = args["file_paths"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if paths.is_empty() {
            return Err(JsonRpcError {
                code: -32602,
                message: "Missing or empty 'file_paths' parameter".into(),
            });
        }

        let mut request = self
            .client
            .post(format!("{}/api/v1/fragments/context", self.server_url))
            .json(&json!({ "file_paths": paths }));
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }
        let response = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(JsonRpcError {
                code: -32000,
                message: format!("API error ({}): {}", status, body),
            });
        }
        let body: Value = response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;

        let checked = body["checked_paths"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_else(|| paths.join(", "));

        let lines: Vec<&str> = body["blocks"]
            .as_array()
            .map(|bs| {
                bs.iter()
                    .filter_map(|b| b.get("text")?.as_str())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let text = if lines.is_empty() {
            format!("No captured knowledge for: {checked}")
        } else {
            format!("{}\n\nChecked: {checked}", lines.join("\n\n"))
        };

        Ok(json!({ "content": [{ "type": "text", "text": text }] }))
    }

    /// `docbrain_commit_capture` — capture intent and reasoning at commit time.
    ///
    /// Creates a knowledge fragment via `POST /api/v1/fragments` with:
    /// - `source_type: commit`
    /// - Content combines intent + diff summary + commit message
    /// - `code_location`: the changed files, which the server reads back as one
    ///   path premise each, so the capture is checked against the files it
    ///   describes instead of ageing silently
    /// - `premises`: anything else the reasoning rests on, passed straight
    ///   through the way `docbrain_annotate` passes its own
    async fn tool_commit_capture(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let gap_id = gap_cluster_id(args)?;
        let intent = args["intent"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'intent' parameter — why was this change made?".into(),
        })?;

        // Input validation
        if intent.trim().is_empty() {
            return Err(JsonRpcError {
                code: -32602,
                message: "intent must not be empty".into(),
            });
        }
        if intent.chars().count() > 10_000 {
            return Err(JsonRpcError {
                code: -32602,
                message: "intent exceeds 10000 character limit".into(),
            });
        }
        let diff_summary = args["diff_summary"].as_str().unwrap_or("");
        if diff_summary.chars().count() > 5_000 {
            return Err(JsonRpcError {
                code: -32602,
                message: "diff_summary exceeds 5000 character limit".into(),
            });
        }
        let commit_message = args["commit_message"].as_str().unwrap_or("");
        if commit_message.chars().count() > 1_000 {
            return Err(JsonRpcError {
                code: -32602,
                message: "commit_message exceeds 1000 character limit".into(),
            });
        }

        let space = args["space"].as_str();

        // Build rich content from all inputs
        let mut content = format!("## Intent\n\n{}", intent);
        if !commit_message.is_empty() {
            content.push_str(&format!("\n\n## Commit Message\n\n{}", commit_message));
        }
        if !diff_summary.is_empty() {
            content.push_str(&format!("\n\n## Changes\n\n{}", diff_summary));
        }

        // Summary: first line of intent, truncated (UTF-8 safe)
        let summary = intent.lines().next().unwrap_or(intent);
        let summary: String = summary.chars().take(200).collect();

        // Build code_location from file paths if provided
        let code_location = args["file_paths"].as_array().and_then(|paths| {
            let file_list: Vec<&str> = paths.iter().take(10).filter_map(|p| p.as_str()).collect();
            if file_list.is_empty() {
                None
            } else {
                Some(file_list.join(", "))
            }
        });

        // Source ID for dedup: hash of commit message + intent
        let mut hasher = Sha256::new();
        hasher.update(commit_message.as_bytes());
        hasher.update(intent.as_bytes());
        let source_id = format!("commit:{}", &hex::encode(hasher.finalize())[..16]);

        let mut body = json!({
            "fragment_type": "decision",
            "summary": summary,
            "content": content,
            "source_type": "commit",
            "source_id": source_id,
            // No confidence field: routing is derived server-side from the
            // capture's anchors (code_location + premises), never from a value
            // the client asserts about its own trustworthiness. Sending one
            // here would read like it controlled whether a human reviews this
            // capture, which it no longer does. Pass `premises` to make the
            // capture falsifiable — that is what earns unreviewed indexing.
        });

        if let Some(loc) = &code_location {
            body["code_location"] = json!(loc);
        }
        if let Some(s) = space {
            body["space"] = json!(s);
        }
        if !commit_message.is_empty() {
            body["source_ref"] = json!(commit_message);
        }
        // Premises the caller declares on top of the ones the server derives
        // from `file_paths` — a version or a config key the reasoning rests on,
        // or a path the commit did not touch but the decision depends on.
        if let Some(premises) = args["premises"].as_array() {
            body["premises"] = json!(premises);
        }
        if let Some(gap) = &gap_id {
            body["gap_cluster_id"] = json!(gap);
        }

        match self.create_fragment(&body).await? {
            Ok(result) => {
                let files_note = match &code_location {
                    Some(loc) => format!(
                        "\nLinked files: {}\nEach is watched — this capture is flagged if one of them moves or disappears.",
                        loc
                    ),
                    None => "\nTip: pass file_paths so this capture is checked against the files it describes; \
                             without them nothing can ever tell whether it still holds."
                        .to_string(),
                };

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "Commit knowledge captured.\n{}{}\n\n\
                             The intent behind this change has been preserved in DocBrain's knowledge base.",
                            capture_report(&result), files_note
                        )
                    }]
                }))
            }
            Err(msg) => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": msg
                }]
            })),
        }
    }

    async fn tool_feedback(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let episode_id = args["episode_id"].as_str().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'episode_id' parameter".into(),
        })?;
        let thumbs_up = args["thumbs_up"].as_bool().ok_or_else(|| JsonRpcError {
            code: -32602,
            message: "Missing 'thumbs_up' parameter (true or false)".into(),
        })?;

        let feedback_score: i8 = if thumbs_up { 1 } else { -1 };

        let mut body = json!({
            "episode_id": episode_id,
            "feedback": feedback_score
        });

        if let Some(reason) = args["reason"].as_str() {
            body["reason"] = json!(reason);
        }
        if let Some(note) = args["note"].as_str() {
            body["note"] = json!(note);
        }

        self.api_call("/api/v1/feedback", &body).await?;

        let label = if thumbs_up {
            "positive (helpful)"
        } else {
            "negative (unhelpful)"
        };

        Ok(json!({
            "content": [{
                "type": "text",
                "text": format!("Feedback recorded: {} for episode {}. Thank you — this helps DocBrain learn.", label, episode_id)
            }]
        }))
    }

    /// Create a fragment via POST /api/v1/fragments.
    ///
    /// Returns `Ok(Ok(json))` on success, `Ok(Err(text))` when the server refused the
    /// capture (any non-2xx: keyless key, rate limit, duplicate, secret, instruction,
    /// not saved), with `text` in the server's own words via `describe_failure`, or
    /// `Err(JsonRpcError)` when the server could not be reached.
    ///
    /// A 500 is never relabeled a duplicate: only the server's `kind` says what a
    /// refusal is.
    async fn create_fragment(&self, body: &Value) -> Result<Result<Value, String>, JsonRpcError> {
        Ok(self
            .api_request(reqwest::Method::POST, "/api/v1/fragments", body)
            .await?
            .map_err(|f| describe_failure(&f)))
    }

    /// One JSON request to the DocBrain API. The outer `Err` is transport (no key, server
    /// unreachable); the inner `Err` is the server's refusal, parsed.
    async fn api_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &Value,
    ) -> Result<Result<Value, CaptureFailure>, JsonRpcError> {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.server_url, path))
            .header(DOCBRAIN_CALLER_HEADER, DOCBRAIN_CALLER_VALUE)
            .json(body);
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }
        let resp = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;
        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;
        if (200..300).contains(&status) {
            if text.trim().is_empty() {
                return Ok(Ok(Value::Null));
            }
            let v = serde_json::from_str(&text).map_err(|e| JsonRpcError {
                code: -32000,
                message: format!("Invalid response: {}", e),
            })?;
            Ok(Ok(v))
        } else {
            Ok(Err(parse_failure(status, &text)))
        }
    }

    async fn tool_revise(&self, args: &Value) -> Result<Value, JsonRpcError> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| JsonRpcError {
                code: -32602,
                message: "id is required".into(),
            })?;
        // The id goes into a URL path: only a UUID-shaped id may.
        if !is_uuid_shaped(id) {
            return Err(JsonRpcError {
                code: -32602,
                message: "id must be a capture id".into(),
            });
        }
        let intent = revise_intent(args).map_err(|message| JsonRpcError {
            code: -32602,
            message,
        })?;
        let (method, path, body, done) = match &intent {
            ReviseIntent::Edit(b) => (
                reqwest::Method::PATCH,
                format!("/api/v1/fragments/{id}"),
                b.clone(),
                "Capture edited and re-indexed",
            ),
            ReviseIntent::Withdraw(r) => (
                reqwest::Method::POST,
                format!("/api/v1/fragments/{id}/withdraw"),
                json!({"reason": r}),
                "Capture withdrawn: it is no longer used in answers",
            ),
        };
        let text = match self.api_request(method, &path, &body).await? {
            Ok(v) => {
                let draft = v
                    .get("carried_by_draft")
                    .and_then(Value::as_str)
                    .map(|d| {
                        format!("\nA composed draft ({d}) still carries its text; withdraw does not change the draft.")
                    })
                    .unwrap_or_default();
                format!("{done}. id {id}{draft}")
            }
            Err(f) => describe_revise_failure(&f),
        };
        Ok(json!({"content": [{"type": "text", "text": text}]}))
    }

    pub async fn api_call(&self, path: &str, body: &Value) -> Result<Value, JsonRpcError> {
        self.post(&self.client, path, body).await.map_err(JsonRpcError::from)
    }

    async fn post(
        &self,
        client: &reqwest::Client,
        path: &str,
        body: &Value,
    ) -> Result<Value, ApiFailure> {
        let url = format!("{}{}", self.server_url, path);

        // Caller header attached on the request builder before auth so the
        // header is present regardless of whether DOCBRAIN_API_KEY is set.
        let mut request = client
            .post(&url)
            .json(body)
            .header(DOCBRAIN_CALLER_HEADER, DOCBRAIN_CALLER_VALUE);
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| ApiFailure {
            status: None,
            body: String::new(),
            message: format!("API call failed: {}", e),
        })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ApiFailure {
                status: Some(status.as_u16()),
                message: format!("API error ({}): {}", status, body),
                body,
            });
        }

        response.json().await.map_err(|e| ApiFailure {
            status: None,
            body: String::new(),
            message: format!("Invalid response: {}", e),
        })
    }
}

/// `docbrain_ask`'s `session_id` description, shared with the docs.
const SESSION_ID_DESCRIPTION: &str = "Continue a conversation: pass only a session_id returned by a previous docbrain_ask, so a follow-up is understood in context. Omit it to start a new conversation. A conversation can be continued by the same DocBrain user (the API key's linked user, or the key itself when it has none).";

/// The tool error for a `session_id` the server will not continue.
const START_NEW_CONVERSATION: &str = "start a new conversation: omit session_id";

/// Appended to every capture tool and to each argument whose text becomes the capture, so the
/// model leaves secrets out before anything leaves the machine. The server's refusal is the
/// backstop; this is where the model reads what to write.
const CAPTURE_SECRET_GUARDRAIL: &str = "Never include a secret value: a password, token, API key, private key, or a connection string or URL with credentials. Write where it lives instead, e.g. 'the database password is in the team vault' or '${DB_PASSWORD}'. DocBrain refuses a capture that holds a secret.";

/// A capture tool's description, or a text argument's, followed by the secret guardrail.
fn with_secret_guardrail(description: &str) -> String {
    format!("{description} {CAPTURE_SECRET_GUARDRAIL}")
}

/// What the model is told when the server refuses a capture for a secret. It names no value:
/// the tool never repeats the text it sent.
const SECRET_REFUSAL_GUIDANCE: &str = "Remove the value and send again; describe where the secret lives instead. Do not encode, split, space out or reword the value to get past the check.";

/// A failed API call, with what the caller may need to classify it.
struct ApiFailure {
    status: Option<u16>,
    body: String,
    message: String,
}

impl ApiFailure {
    /// The server refused the session itself: someone else's (403), expired (409),
    /// or not an id at all (400 "Invalid session_id"). Any other failure — an
    /// over-long question is also a 400 — is not about the session.
    fn names_a_dead_session(&self) -> bool {
        match self.status {
            Some(403 | 409) => true,
            Some(400) => self.body.starts_with("Invalid session_id"),
            _ => false,
        }
    }
}

impl From<ApiFailure> for JsonRpcError {
    fn from(f: ApiFailure) -> Self {
        JsonRpcError {
            code: -32000,
            message: f.message,
        }
    }
}

fn is_uuid_shaped(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    candidate.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

/// The optional `gap_id` argument, sent as `gap_cluster_id`. Absent is fine; present
/// must be a UUID-shaped string.
fn gap_cluster_id(args: &Value) -> Result<Option<String>, JsonRpcError> {
    match args.get("gap_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(g)) if is_uuid_shaped(g) => Ok(Some(g.clone())),
        Some(_) => Err(JsonRpcError {
            code: -32602,
            message: "gap_id must be an id".into(),
        }),
    }
}

/// `Fragment ID: .. | Status: ..` plus one `Warning: ` line per server warning.
fn capture_report(result: &Value) -> String {
    let id = result["id"].as_str().unwrap_or("unknown");
    let status = result["status"].as_str().unwrap_or("unknown");
    let mut out = format!("Fragment ID: {id} | Status: {status}");
    for w in result["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        out.push_str("\nWarning: ");
        out.push_str(w);
    }
    out
}

#[derive(Debug)]
enum ReviseIntent {
    Edit(Value),
    Withdraw(String),
}

fn revise_intent(args: &Value) -> Result<ReviseIntent, String> {
    let text = args.get("text").and_then(Value::as_str);
    let premises = args.get("premises").filter(|v| v.is_array());
    let reason = args.get("withdraw_reason").and_then(Value::as_str);
    match (text.is_some() || premises.is_some(), reason) {
        (true, Some(_)) => Err("choose one: edit (text/premises) or withdraw (withdraw_reason), not both".into()),
        (false, None) => Err("nothing to do: give text and/or premises to edit, or withdraw_reason to withdraw".into()),
        (false, Some(r)) if r.trim().is_empty() => Err("withdraw_reason cannot be blank: it is shown with the capture".into()),
        (false, Some(r)) if r.chars().count() > WITHDRAW_REASON_MAX_CHARS => Err("withdraw_reason exceeds 1000 characters".into()),
        (false, Some(r)) => Ok(ReviseIntent::Withdraw(r.trim().to_string())),
        (true, None) => {
            if text.is_some_and(|t| t.chars().count() > 10_000) {
                return Err("text exceeds 10000 characters".into());
            }
            let mut body = serde_json::Map::new();
            if let Some(t) = text {
                body.insert(
                    "summary".into(),
                    Value::String(t.lines().next().unwrap_or("").chars().take(200).collect()),
                );
                body.insert("content".into(), Value::String(t.to_string()));
            }
            if let Some(p) = premises {
                body.insert("premises".into(), p.clone());
            }
            Ok(ReviseIntent::Edit(Value::Object(body)))
        }
    }
}

/// A refusal from the DocBrain API: `{"error", "kind", "id"?, "retry_after_secs"?}`, or a plain body.
#[derive(Debug)]
struct CaptureFailure {
    status: u16,
    kind: String,
    message: String,
    id: Option<String>,
    retry_after_secs: Option<u64>,
}

fn parse_failure(status: u16, body: &str) -> CaptureFailure {
    match serde_json::from_str::<Value>(body) {
        Ok(v) if v.get("error").is_some() => CaptureFailure {
            status,
            kind: v["kind"].as_str().unwrap_or("unknown").to_string(),
            message: v["error"].as_str().unwrap_or("").to_string(),
            id: v.get("id").and_then(Value::as_str).map(str::to_string),
            retry_after_secs: v.get("retry_after_secs").and_then(Value::as_u64),
        },
        _ => CaptureFailure {
            status,
            kind: "unknown".into(),
            message: body.trim().to_string(),
            id: None,
            retry_after_secs: None,
        },
    }
}

/// A refusal of a create, in the server's words.
fn describe_failure(f: &CaptureFailure) -> String {
    describe_refusal(f, "Not saved")
}

/// A refusal of an edit or withdraw: "Not changed", so a refused withdraw is never
/// read as "saved" or as "still live".
fn describe_revise_failure(f: &CaptureFailure) -> String {
    describe_refusal(f, "Not changed")
}

/// What an agent does when the server could not check the text.
const NOT_CHECKED_GUIDANCE: &str =
    "Not saved: DocBrain could not check this text; try once more, and if it is refused again, report it and stop";
/// What an agent does when the capture changed under its edit.
const CAPTURE_CHANGED_GUIDANCE: &str = "Not saved: the capture changed while you edited; read it again and send your edit";

/// Any 429 is a rate limit, kind or not: the generic limiter answers plain text.
fn describe_refusal(f: &CaptureFailure, verb: &str) -> String {
    if f.kind == "capture_limit" || f.status == 429 {
        let wait = f
            .retry_after_secs
            .map(|n| format!(" (retry in {n}s)"))
            .unwrap_or_default();
        return format!("{verb}: {}{wait} — do not retry now; report this and stop.", f.message);
    }
    match f.kind.as_str() {
        "already_captured" => format!(
            "{verb}: DocBrain already has this capture ({}). Use docbrain_revise to change it.",
            f.id.as_deref().unwrap_or("id not given")
        ),
        // Any secret refusal kind: the server's words, then what to do. Only the server's
        // message is relayed, never the text that was sent.
        kind if kind.starts_with("secret_") => {
            format!("{verb} ({}): {}\n{SECRET_REFUSAL_GUIDANCE}", f.status, f.message)
        }
        // One retry, then stop, so an agent never loops into the cooldown.
        "not_checked" => format!("{NOT_CHECKED_GUIDANCE} ({}: {})", f.status, f.message),
        "capture_changed" => format!("{CAPTURE_CHANGED_GUIDANCE} ({}: {})", f.status, f.message),
        _ => format!("{verb} ({}): {}", f.status, f.message),
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    // `--version` and `--help` used to be unreachable: startup validation ran
    // before any argument handling, so the binary exited 1 with
    // "DOCBRAIN_API_KEY is not set" instead of printing its version. Asking a
    // binary what it is must not require credentials.

    #[test]
    fn version_flag_is_recognised() {
        assert_eq!(parse_cli(&["--version".to_string()]), CliAction::Version);
        assert_eq!(parse_cli(&["-V".to_string()]), CliAction::Version);
    }

    #[test]
    fn help_flag_is_recognised() {
        assert_eq!(parse_cli(&["--help".to_string()]), CliAction::Help);
        assert_eq!(parse_cli(&["-h".to_string()]), CliAction::Help);
    }

    #[test]
    fn no_arguments_serves_the_protocol() {
        assert_eq!(parse_cli(&[]), CliAction::Serve);
    }

    /// An unrecognised flag is surfaced as `Unknown` so the binary can warn
    /// about it. BLINDSPOT (pre-push): it must NOT cause an exit —
    /// `npm-mcp/run.js` forwards `process.argv.slice(2)`, so exiting would
    /// break any MCP host that passes an argument, which worked before
    /// argument handling existed. See the dispatch in `main.rs`.
    #[test]
    fn unknown_flag_is_reported() {
        assert_eq!(
            parse_cli(&["--frobnicate".to_string()]),
            CliAction::Unknown("--frobnicate".to_string())
        );
    }

    /// A JSON-RPC host launches the binary with no flags; extra non-flag
    /// arguments are not a thing it does, but they must not be mistaken for
    /// flags either.
    #[test]
    fn bare_argument_is_reported_as_unknown() {
        assert_eq!(
            parse_cli(&["serve".to_string()]),
            CliAction::Unknown("serve".to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server() -> McpServer {
        McpServer {
            server_url: "http://localhost:9999".to_string(),
            api_key: Some("test_key".to_string()),
            client: reqwest::Client::new(),
            ask_client: reqwest::Client::new(),
            role: std::sync::OnceLock::new(),
        }
    }

    #[test]
    fn tools_list_includes_new_tools() {
        let server = make_server();
        let list = server.handle_tools_list();
        let tools = list["tools"].as_array().expect("tools should be an array");
        let names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();

        assert!(
            names.contains(&"docbrain_annotate"),
            "missing docbrain_annotate"
        );
        assert!(
            names.contains(&"docbrain_suggest_capture"),
            "missing docbrain_suggest_capture"
        );
        assert!(
            names.contains(&"docbrain_commit_capture"),
            "missing docbrain_commit_capture"
        );
        assert!(
            names.contains(&"docbrain_context"),
            "missing docbrain_context"
        );
        // Verify total tool count: 11 + docbrain_revise = 12
        assert_eq!(
            names.len(),
            12,
            "expected 12 tools, got {}: {:?}",
            names.len(),
            names
        );
    }

    /// B1's bar is "in words an agent will act on". Naming the accepted form
    /// alone doesn't rule out `path:42` — a coding agent forwarding a
    /// compiler/linter/stack-trace location verbatim is the single most
    /// likely call it makes, and it reads the `file_paths` property
    /// description (not just the top-level description) when building
    /// arguments, so both must carry the rule.
    #[test]
    fn context_description_explicitly_rejects_a_line_suffixed_path() {
        let server = make_server();
        let list = server.handle_tools_list();
        let tools = list["tools"].as_array().unwrap();
        let context = tools
            .iter()
            .find(|t| t["name"] == "docbrain_context")
            .unwrap();
        let description = context["description"].as_str().unwrap();
        let file_paths_description =
            context["inputSchema"]["properties"]["file_paths"]["description"]
                .as_str()
                .unwrap();

        for (label, desc) in [
            ("tool description", description),
            ("file_paths schema description", file_paths_description),
        ] {
            assert!(
                desc.contains("session.rs:42"),
                "{label} must show a rejected, line-suffixed example: {desc}"
            );
            assert!(
                desc.to_lowercase().contains("reject") || desc.to_lowercase().contains("wrong"),
                "{label} must say the line-suffixed form is wrong, not just show it: {desc}"
            );
        }
    }

    #[test]
    fn annotate_tool_schema_has_required_fields() {
        let server = make_server();
        let list = server.handle_tools_list();
        let tools = list["tools"].as_array().unwrap();
        let annotate = tools
            .iter()
            .find(|t| t["name"] == "docbrain_annotate")
            .unwrap();
        let required = annotate["inputSchema"]["required"].as_array().unwrap();
        let req_strs: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"file_path"));
        assert!(req_strs.contains(&"annotation"));

        // premises is optional metadata, not required on the wire.
        assert!(!req_strs.contains(&"premises"));
        let premises_schema = &annotate["inputSchema"]["properties"]["premises"];
        assert_eq!(premises_schema["type"], "array");
        let item_required = premises_schema["items"]["required"]
            .as_array()
            .expect("premises items should declare required fields");
        let item_req_strs: Vec<&str> = item_required.iter().filter_map(|v| v.as_str()).collect();
        assert!(item_req_strs.contains(&"premise_type"));
        assert!(item_req_strs.contains(&"expression"));
    }

    #[tokio::test]
    async fn annotate_validates_missing_file_path() {
        let server = make_server();
        let args = json!({ "annotation": "some knowledge" });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("file_path"),
            "error should mention file_path: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn annotate_validates_missing_annotation() {
        let server = make_server();
        let args = json!({ "file_path": "src/main.rs" });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("annotation"),
            "error should mention annotation: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn annotate_validates_annotation_length() {
        let server = make_server();
        let long = "x".repeat(10_001);
        let args = json!({ "file_path": "src/main.rs", "annotation": long });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.message.contains("10000"),
            "error should mention limit: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn annotate_validates_invalid_fragment_type() {
        let server = make_server();
        let args = json!({
            "file_path": "src/main.rs",
            "annotation": "test",
            "fragment_type": "invalid_type"
        });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("Invalid fragment_type"));
    }

    #[test]
    fn annotate_computes_code_hash_for_drift() {
        // Verify SHA-256 hash computation matches expected output
        let code = "fn main() { println!(\"hello\"); }";
        let mut hasher = Sha256::new();
        hasher.update(code.as_bytes());
        let hash = hex::encode(hasher.finalize());

        // Hash should be a 64-char hex string
        assert_eq!(hash.len(), 64);
        // Same input → same hash (deterministic)
        let mut hasher2 = Sha256::new();
        hasher2.update(code.as_bytes());
        assert_eq!(hex::encode(hasher2.finalize()), hash);

        // Different input → different hash
        let mut hasher3 = Sha256::new();
        hasher3.update("fn main() { }".as_bytes());
        assert_ne!(hex::encode(hasher3.finalize()), hash);
    }

    /// The commit tool takes premises the way the annotate tool does. Without
    /// it a commit capture could only ever carry what the server derives from
    /// its file list, and a reasoning that rests on a version or a config key
    /// would have nowhere to say so.
    #[test]
    fn commit_capture_accepts_premises_like_annotate_does() {
        let server = make_server();
        let list = server.handle_tools_list();
        let tools = list["tools"].as_array().unwrap();
        let schema_of = |name: &str| {
            tools
                .iter()
                .find(|t| t["name"] == name)
                .unwrap_or_else(|| panic!("{name} missing from tools/list"))["inputSchema"]
                ["properties"]["premises"]
                .clone()
        };

        let commit = schema_of("docbrain_commit_capture");
        assert!(
            commit.is_object(),
            "docbrain_commit_capture must declare a premises argument"
        );
        assert_eq!(
            commit["items"], schema_of("docbrain_annotate")["items"],
            "one premise shape across the capture tools, not two"
        );
    }

    /// The file list is what makes a commit capture checkable, so the argument
    /// has to say so where an agent reads it — the property description, not
    /// only the tool's own.
    #[test]
    fn commit_capture_file_paths_description_says_it_is_what_gets_checked() {
        let server = make_server();
        let list = server.handle_tools_list();
        let tools = list["tools"].as_array().unwrap();
        let description = tools
            .iter()
            .find(|t| t["name"] == "docbrain_commit_capture")
            .unwrap()["inputSchema"]["properties"]["file_paths"]["description"]
            .as_str()
            .unwrap();
        assert!(
            description.contains("premise"),
            "an agent deciding whether to send file_paths must be told what they buy: {description}"
        );
    }

    #[tokio::test]
    async fn commit_capture_validates_missing_intent() {
        let server = make_server();
        let args = json!({ "diff_summary": "changed something" });
        let result = server.tool_commit_capture(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("intent"));
    }

    #[tokio::test]
    async fn commit_capture_validates_intent_length() {
        let server = make_server();
        let long = "x".repeat(10_001);
        let args = json!({ "intent": long });
        let result = server.tool_commit_capture(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("10000"));
    }

    #[tokio::test]
    async fn commit_capture_validates_diff_summary_length() {
        let server = make_server();
        let long = "x".repeat(5_001);
        let args = json!({ "intent": "fix bug", "diff_summary": long });
        let result = server.tool_commit_capture(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("5000"));
    }

    #[tokio::test]
    async fn suggest_capture_validates_missing_file_path() {
        let server = make_server();
        let args = json!({});
        let result = server.tool_suggest_capture(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.message.contains("file_path"));
    }

    /// Previously verified only by a throwaway probe (added, run, then
    /// deleted) — a deleted test protects nothing on the next change.
    #[tokio::test]
    async fn context_validates_missing_file_paths() {
        let server = make_server();
        let args = json!({});
        let result = server.tool_context(&args).await;
        let err = result.unwrap_err();
        // `is_err()` alone proves nothing here: make_server points at an
        // unreachable port, so a request that gets PAST the guard also returns
        // Err — from the network. The code and the message are what discriminate.
        assert_eq!(err.code, -32602, "invalid params, not a transport failure");
        assert!(err.message.contains("file_paths"));
    }

    #[tokio::test]
    async fn context_rejects_empty_file_paths() {
        let server = make_server();
        let args = json!({ "file_paths": [] });
        let result = server.tool_context(&args).await;
        let err = result.unwrap_err();
        assert_eq!(err.code, -32602, "invalid params, not a transport failure");
        assert!(err.message.contains("empty"));
    }

    #[test]
    fn commit_capture_source_id_is_deterministic() {
        // Same commit_message + intent → same source_id (idempotency)
        let compute = |msg: &str, intent: &str| -> String {
            let mut hasher = Sha256::new();
            hasher.update(msg.as_bytes());
            hasher.update(intent.as_bytes());
            format!("commit:{}", &hex::encode(hasher.finalize())[..16])
        };

        let id1 = compute("fix: null check", "prevent crash on empty input");
        let id2 = compute("fix: null check", "prevent crash on empty input");
        assert_eq!(id1, id2, "same input should produce same source_id");

        let id3 = compute("fix: null check", "different intent");
        assert_ne!(
            id1, id3,
            "different input should produce different source_id"
        );
    }

    #[test]
    fn initialize_response_is_valid() {
        let server = make_server();
        let init = server.handle_initialize();
        assert_eq!(init["protocolVersion"], "2024-11-05");
        assert!(init["capabilities"]["tools"].is_object());
        assert_eq!(init["serverInfo"]["name"], "docbrain");
    }

    #[tokio::test]
    async fn annotate_rejects_empty_file_path() {
        let server = make_server();
        let args = json!({ "file_path": "  ", "annotation": "some knowledge" });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("must not be empty"));
    }

    #[tokio::test]
    async fn annotate_rejects_empty_annotation() {
        let server = make_server();
        let args = json!({ "file_path": "src/main.rs", "annotation": "" });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("must not be empty"));
    }

    #[tokio::test]
    async fn commit_capture_rejects_empty_intent() {
        let server = make_server();
        let args = json!({ "intent": "   " });
        let result = server.tool_commit_capture(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("must not be empty"));
    }

    #[test]
    fn summary_truncation_handles_multibyte_utf8() {
        // Ensure chars().take(200) doesn't panic on multi-byte content
        let emoji_str = "🎉".repeat(300); // 300 emoji, each 4 bytes
        let summary: String = emoji_str.chars().take(200).collect();
        assert_eq!(summary.chars().count(), 200);

        let cjk_str = "文".repeat(250);
        let summary: String = cjk_str.chars().take(200).collect();
        assert_eq!(summary.chars().count(), 200);
    }

    #[test]
    fn revise_is_listed_with_its_schema() {
        let s = make_server();
        let list = s.handle_tools_list();
        let tools = list["tools"].as_array().expect("tools");
        assert_eq!(tools.len(), 12);
        let revise = tools.iter().find(|t| t["name"] == "docbrain_revise").expect("revise");
        assert_eq!(revise["inputSchema"]["required"], serde_json::json!(["id"]));
        for p in ["text", "premises", "withdraw_reason"] {
            assert!(revise["inputSchema"]["properties"].get(p).is_some(), "{p}");
        }
        for t in ["docbrain_annotate", "docbrain_commit_capture"] {
            let tool = tools.iter().find(|x| x["name"] == t).expect(t);
            assert!(tool["inputSchema"]["properties"].get("gap_id").is_some(), "{t} takes gap_id");
        }
    }

    #[test]
    fn revise_needs_exactly_one_intent() {
        assert!(revise_intent(&serde_json::json!({"id": "x"})).is_err(), "nothing to do");
        assert!(revise_intent(&serde_json::json!({"id": "x", "text": "t", "withdraw_reason": "r"})).is_err(), "both");
        assert!(matches!(revise_intent(&serde_json::json!({"id": "x", "withdraw_reason": "wrong"})), Ok(ReviseIntent::Withdraw(_))));
        assert!(matches!(revise_intent(&serde_json::json!({"id": "x", "premises": []})), Ok(ReviseIntent::Edit(_))));
        assert!(revise_intent(&serde_json::json!({"id": "x", "withdraw_reason": "   "})).is_err(), "a blank reason");
    }

    #[test]
    fn refusals_are_reported_in_the_servers_words() {
        let limit = CaptureFailure { status: 429, kind: "capture_limit".into(), message: "capture limit reached: 60 an hour".into(), id: None, retry_after_secs: Some(17) };
        let text = describe_failure(&limit);
        assert!(text.contains("capture limit reached: 60 an hour"));
        assert!(text.contains("do not retry"), "the skill stops, never loops");
        assert!(text.contains("retry in 17s"), "{text}");
        let dup = CaptureFailure { status: 409, kind: "already_captured".into(), message: "already captured: abc".into(), id: Some("abc".into()), retry_after_secs: None };
        assert!(describe_failure(&dup).contains("abc"));
    }

    #[test]
    fn a_json_refusal_body_parses_and_a_plain_one_falls_back() {
        let f = parse_failure(422, r#"{"error":"not saved: secret (github_pat)","kind":"secret_in_capture","rule":"github_pat"}"#);
        assert_eq!((f.status, f.kind.as_str()), (422, "secret_in_capture"));
        let p = parse_failure(500, "Internal server error (ref: 1)");
        assert_eq!(p.kind, "unknown");
        assert_eq!(p.message, "Internal server error (ref: 1)");
    }

    #[test]
    fn gap_id_must_be_a_uuid() {
        assert!(gap_cluster_id(&serde_json::json!({})).expect("absent is fine").is_none());
        let ok = gap_cluster_id(&serde_json::json!({"gap_id": "7f3c0000-0000-0000-0000-000000000a91"}));
        assert_eq!(ok.expect("uuid").as_deref(), Some("7f3c0000-0000-0000-0000-000000000a91"));
        for bad in ["nope", "", "7f3c0000-0000-0000-0000-000000000a9", "7f3c0000-0000-0000-0000-000000000a9g"] {
            let e = gap_cluster_id(&serde_json::json!({"gap_id": bad})).expect_err(bad);
            assert_eq!(e.code, -32602);
            assert_eq!(e.message, "gap_id must be an id");
        }
        assert!(gap_cluster_id(&serde_json::json!({"gap_id": 5})).is_err(), "not a string");
    }

    #[test]
    fn capture_success_lists_warnings_and_no_action() {
        let t = capture_report(&serde_json::json!({"id": "i1", "status": "indexed", "routed_action": "x", "warnings": ["w one", "w two"]}));
        assert!(t.contains("Fragment ID: i1 | Status: indexed"));
        assert!(!t.contains("Action:"));
        assert!(t.contains("Warning: w one") && t.contains("Warning: w two"));
    }

    #[test]
    fn a_refused_revise_never_says_not_saved() {
        let edit = parse_failure(409, r#"{"error":"this capture is withdrawn","kind":"withdrawn"}"#);
        let t = describe_revise_failure(&edit);
        assert!(t.starts_with("Not changed (409)") && !t.contains("Not saved"), "{t}");
        let wd = parse_failure(409, r#"{"error":"already withdrawn","kind":"already_withdrawn"}"#);
        let t = describe_revise_failure(&wd);
        assert!(t.starts_with("Not changed") && t.contains("already withdrawn") && !t.contains("Not saved"), "{t}");
    }

    #[test]
    fn a_kindless_429_stops_the_agent() {
        let f = parse_failure(429, "Too many requests");
        for t in [describe_failure(&f), describe_revise_failure(&f)] {
            assert!(t.contains("Too many requests") && t.contains("do not retry now"), "{t}");
        }
        let body = parse_failure(429, r#"{"error":"capture limit reached","kind":"capture_limit","retry_after_secs":17}"#);
        assert_eq!(body.retry_after_secs, Some(17));
    }

    /// Where the model reads a capture tool — its description and the description of each
    /// argument whose text is stored — it is told to leave secret values out. Every listed
    /// tool is classified below, so a new tool fails here until someone decides whether text
    /// written into it becomes a capture other people can retrieve.
    #[test]
    fn every_capture_tool_and_its_text_arguments_carry_the_secret_guardrail() {
        for words in ["Never include a secret value", "Write where it lives instead", "refuses a capture that holds a secret"] {
            assert!(CAPTURE_SECRET_GUARDRAIL.contains(words), "{words}");
        }
        // (tool, arguments whose text is stored in the capture, every other argument)
        const CAPTURE_TOOLS: &[(&str, &[&str], &[&str])] = &[
            (
                "docbrain_annotate",
                &["annotation"],
                &["file_path", "line_range", "fragment_type", "code_snippet", "space", "gap_id", "premises"],
            ),
            (
                "docbrain_commit_capture",
                &["intent", "diff_summary", "commit_message"],
                &["file_paths", "space", "gap_id", "premises"],
            ),
            ("docbrain_revise", &["text", "withdraw_reason"], &["id", "premises"]),
        ];
        const OTHER_TOOLS: &[&str] = &[
            "docbrain_ask",
            "docbrain_incident",
            "docbrain_freshness",
            "docbrain_autopilot_gaps",
            "docbrain_autopilot_generate",
            "docbrain_autopilot_summary",
            "docbrain_suggest_capture",
            "docbrain_context",
            "docbrain_feedback",
        ];
        let list = make_server().handle_tools_list();
        let mut capture_tools_seen = Vec::new();
        for tool in list["tools"].as_array().expect("tools") {
            let name = tool["name"].as_str().expect("a tool has a name");
            if OTHER_TOOLS.contains(&name) {
                continue;
            }
            let (_, text, other) = CAPTURE_TOOLS.iter().find(|(n, ..)| *n == name).unwrap_or_else(|| {
                panic!("{name} is not classified: if text written into it becomes a capture, add it to CAPTURE_TOOLS, otherwise to OTHER_TOOLS")
            });
            let description = tool["description"].as_str().unwrap_or_default();
            assert!(description.contains(CAPTURE_SECRET_GUARDRAIL), "{name}: {description}");
            let properties = tool["inputSchema"]["properties"].as_object().expect("properties");
            for (arg, schema) in properties {
                let arg_description = schema["description"].as_str().unwrap_or_default();
                if text.contains(&arg.as_str()) {
                    assert!(arg_description.contains(CAPTURE_SECRET_GUARDRAIL), "{name}.{arg}: {arg_description}");
                } else {
                    assert!(other.contains(&arg.as_str()), "{name}.{arg} is not classified: is its text stored in the capture?");
                }
            }
            for arg in *text {
                assert!(properties.contains_key(*arg), "{name} no longer takes {arg}");
            }
            capture_tools_seen.push(name);
        }
        assert_eq!(capture_tools_seen.len(), CAPTURE_TOOLS.len(), "every capture tool is listed: {capture_tools_seen:?}");
    }

    /// A refusal for a secret tells the model what to do next and how not to get past the
    /// check, keeps the server's words, and relays nothing but the server's message: a body
    /// that names a rule, or one that carries a field the server never sends, does not put a
    /// value in front of the model.
    #[test]
    fn a_secret_refusal_says_what_to_do_and_never_carries_the_value() {
        let value = "Synthetic-Pass-4417";
        for body in [
            r#"{"error":"not saved: the text contains what looks like a secret (generic_password); remove it and send again","kind":"secret_in_capture","rule":"generic_password","matched":"Synthetic-Pass-4417"}"#,
            r#"{"error":"not saved: the content holds what looks like a password; remove it and send again","kind":"secret_in_capture","secret_kind":"password","field":"content"}"#,
        ] {
            let f = parse_failure(422, body);
            for t in [describe_failure(&f), describe_revise_failure(&f)] {
                assert!(t.contains(SECRET_REFUSAL_GUIDANCE), "{t}");
                assert!(t.contains("describe where the secret lives instead"), "{t}");
                assert!(t.contains("Do not encode, split, space out or reword the value to get past the check"), "{t}");
                assert!(t.contains(&f.message), "the server's words are kept: {t}");
                assert!(!t.contains(value), "never the value: {t}");
            }
        }
        for body in [
            r#"{"error":"not saved: the text reads as an instruction to DocBrain","kind":"instruction_in_capture"}"#,
            r#"{"error":"text is blank","kind":"invalid_request"}"#,
        ] {
            let t = describe_failure(&parse_failure(422, body));
            assert!(!t.contains(SECRET_REFUSAL_GUIDANCE), "only a secret refusal gets the secret guidance: {t}");
        }
    }

    /// A capture the server could not check, or one that changed under an edit, gets
    /// one plain next step, so an IDE agent neither loops into the cooldown nor resends stale text.
    #[test]
    fn a_not_checked_or_changed_capture_says_what_to_do_once() {
        let f = parse_failure(503, r#"{"error":"not saved: the text could not be checked; try again","kind":"not_checked"}"#);
        for t in [describe_failure(&f), describe_revise_failure(&f)] {
            assert!(
                t.contains("Not saved: DocBrain could not check this text; try once more, and if it is refused again, report it and stop"),
                "{t}"
            );
        }
        let f = parse_failure(
            409,
            r#"{"error":"not saved: the capture changed while you edited; send again","kind":"capture_changed"}"#,
        );
        for t in [describe_failure(&f), describe_revise_failure(&f)] {
            assert!(t.contains("Not saved: the capture changed while you edited; read it again and send your edit"), "{t}");
        }
    }

    /// The examples in the tool descriptions are invented, not names that read like a real
    /// organization's systems.
    #[test]
    fn tool_descriptions_use_invented_examples() {
        let text = serde_json::to_string(&make_server().handle_tools_list()).expect("json");
        // The examples are invented, never a deployment's own service names.
        for invented in ["payments service logs", "production database access for engineers", "nightly batch job deployment", "every engineer can deploy to production"] {
            assert!(text.contains(invented), "{invented}");
        }
    }
}
