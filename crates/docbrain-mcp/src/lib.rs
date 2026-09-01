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
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

/// HTTP header name identifying this binary as the caller to DocBrain.
///
/// DocBrain's `/api/v1/ask` handler reads `X-DocBrain-Caller` to set the
/// request's `Surface` (computed server-side).
/// `mcp-host` -> `Surface::McpHost` -> `tools_enabled` defaults to FALSE so we
/// don't double-fetch live data the MCP host already has.
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
                    "description": "Ask a question about your organization's internal documentation. Uses hybrid semantic + keyword search across Confluence, runbooks, and captured Slack/GitHub threads. Preserves specific service names, team names, and tool names for precise retrieval. Corrects false premises (e.g. if you assume engineers have admin EKS access, it will correct you). Returns a direct, human-like answer — never says 'according to the documentation'. Cites up to 5 real source documents with URLs. For questions outside the knowledge base (HR policies, general engineering topics not documented internally), clearly says so rather than guessing. Uses session memory to resolve follow-up questions in context.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "question": {
                                "type": "string",
                                "description": "The question to ask. Use specific service/team/tool names for best results (e.g. 'Canal V2 logs', 'EKS access for engineers', 'Airflow DAG deployment via ArgoCD')."
                            },
                            "session_id": {
                                "type": "string",
                                "description": "UUID to group follow-up questions into a conversation. Omit for standalone questions."
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
                    "description": "Capture knowledge linked to a specific code location. Creates a knowledge fragment tied to a file and line range. Use this when you encounter important decisions, caveats, or context in code that should be preserved as organizational knowledge.",
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
                                "description": "The knowledge to capture — a decision, fact, caveat, procedure, or context (max 10000 chars)."
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
                    "name": "docbrain_commit_capture",
                    "description": "Capture knowledge from a commit — why was this change made? Creates a knowledge fragment from the commit intent, grounding it in the diff and commit message. Use this at commit time to preserve the reasoning behind changes before it's forgotten.",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "diff_summary": {
                                "type": "string",
                                "description": "Summary of what changed in the diff (max 5000 chars). Can be a condensed diff or a human-readable summary of files/functions changed."
                            },
                            "commit_message": {
                                "type": "string",
                                "description": "The commit message (max 1000 chars)."
                            },
                            "intent": {
                                "type": "string",
                                "description": "Why was this change made? The reasoning, trade-offs, alternatives considered, or decisions that led to this commit (max 10000 chars)."
                            },
                            "file_paths": {
                                "type": "array",
                                "items": { "type": "string" },
                                "description": "List of files changed in the commit. Used to link the fragment to code locations."
                            },
                            "space": {
                                "type": "string",
                                "description": "Documentation space to associate with. Optional."
                            }
                        },
                        "required": ["intent"]
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
            "docbrain_commit_capture" => self.tool_commit_capture(arguments).await,
            "docbrain_feedback" => self.tool_feedback(arguments).await,
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

        let body = json!({ "question": question });
        let response = self.api_call("/api/v1/ask", &body).await?;

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

        let mut text = answer.to_string();
        if !sources.is_empty() {
            text.push_str("\n\nSources:\n");
            text.push_str(&sources.join("\n"));
        }
        if let Some(episode_id) = response["episode_id"].as_str() {
            text.push_str(&format!(
                "\n\n<!-- episode_id: {} — pass to docbrain_feedback to rate this answer -->",
                episode_id
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

        let mut text = format!("**INCIDENT RESPONSE**\n\n{}", answer);
        if !sources.is_empty() {
            text.push_str("\n\nRunbooks & Sources:\n");
            text.push_str(&sources.join("\n"));
        }
        if let Some(episode_id) = response["episode_id"].as_str() {
            text.push_str(&format!(
                "\n\n<!-- episode_id: {} — pass to docbrain_feedback to rate this response -->",
                episode_id
            ));
        }

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
            "confidence": 0.85,
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

        match self.create_fragment(&body).await? {
            Ok(result) => {
                let id = result["id"].as_str().unwrap_or("unknown");
                let status = result["status"].as_str().unwrap_or("unknown");
                let action = result["routed_action"].as_str().unwrap_or("unknown");

                let drift_note = if code_hash.is_some() {
                    " Code hash saved — annotation will be flagged if the code changes."
                } else {
                    " Tip: pass code_snippet to enable drift detection."
                };

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "Knowledge captured: {} annotation at {}\nFragment ID: {} | Status: {} | Action: {}\n{}",
                            fragment_type, code_location, id, status, action, drift_note
                        )
                    }]
                }))
            }
            Err(_) => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": format!("Annotation already exists for {} — no duplicate created.", code_location)
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

    /// `docbrain_commit_capture` — capture intent and reasoning at commit time.
    ///
    /// Creates a knowledge fragment via `POST /api/v1/fragments` with:
    /// - `source_type: commit`
    /// - Content combines intent + diff summary + commit message
    async fn tool_commit_capture(&self, args: &Value) -> Result<Value, JsonRpcError> {
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
            "confidence": 0.80,
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

        match self.create_fragment(&body).await? {
            Ok(result) => {
                let id = result["id"].as_str().unwrap_or("unknown");
                let status = result["status"].as_str().unwrap_or("unknown");
                let action = result["routed_action"].as_str().unwrap_or("unknown");

                let files_note = match &code_location {
                    Some(loc) => format!("\nLinked files: {}", loc),
                    None => String::new(),
                };

                Ok(json!({
                    "content": [{
                        "type": "text",
                        "text": format!(
                            "Commit knowledge captured.\nFragment ID: {} | Status: {} | Action: {}{}\n\n\
                             The intent behind this change has been preserved in DocBrain's knowledge base.",
                            id, status, action, files_note
                        )
                    }]
                }))
            }
            Err(_msg) => Ok(json!({
                "content": [{
                    "type": "text",
                    "text": "This commit's knowledge has already been captured — no duplicate created."
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

    /// Create a fragment via POST /api/v1/fragments, handling duplicate source_id gracefully.
    ///
    /// Returns `Ok(Ok(json))` on success, `Ok(Err(message))` on duplicate (409/unique violation),
    /// or `Err(JsonRpcError)` on real failures.
    async fn create_fragment(&self, body: &Value) -> Result<Result<Value, String>, JsonRpcError> {
        let url = format!("{}/api/v1/fragments", self.server_url);

        let mut request = self.client.post(&url).json(body);
        if let Some(ref key) = self.api_key {
            request = request.header("Authorization", format!("Bearer {}", key));
        }

        let response = request.send().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("API call failed: {}", e),
        })?;

        let status = response.status();

        // 409 Conflict or 500 with unique violation = duplicate fragment
        if status.as_u16() == 409 {
            return Ok(Err(
                "Fragment already exists for this annotation (duplicate source_id).".into(),
            ));
        }
        if status.as_u16() == 500 {
            let text = response.text().await.unwrap_or_default();
            if text.contains("unique") || text.contains("duplicate") || text.contains("23505") {
                return Ok(Err(
                    "Fragment already exists for this annotation (duplicate source_id).".into(),
                ));
            }
            return Err(JsonRpcError {
                code: -32000,
                message: format!("API error ({}): {}", status, text),
            });
        }

        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(JsonRpcError {
                code: -32000,
                message: format!("API error ({}): {}", status, text),
            });
        }

        let json = response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })?;
        Ok(Ok(json))
    }

    pub async fn api_call(&self, path: &str, body: &Value) -> Result<Value, JsonRpcError> {
        let url = format!("{}{}", self.server_url, path);

        // Caller header attached on the request builder before auth so the
        // header is present regardless of whether DOCBRAIN_API_KEY is set.
        let mut request = self
            .client
            .post(&url)
            .json(body)
            .header(DOCBRAIN_CALLER_HEADER, DOCBRAIN_CALLER_VALUE);
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

        response.json().await.map_err(|e| JsonRpcError {
            code: -32000,
            message: format!("Invalid response: {}", e),
        })
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
        // Verify total tool count: 7 original + 3 new = 10
        assert_eq!(
            names.len(),
            10,
            "expected 10 tools, got {}: {:?}",
            names.len(),
            names
        );
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
}
