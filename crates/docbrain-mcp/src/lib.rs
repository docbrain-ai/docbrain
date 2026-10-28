// SPDX-License-Identifier: MIT
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub mod backend;
pub mod http;
use backend::{ApiPath, ApiRequest, Backend, HttpBackend, Method, is_uuid_shaped};

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
    Serve {
        /// The file `--docbrain-env` names, loaded before anything reads the
        /// environment. No other env file is ever read (#603).
        docbrain_env: Option<std::path::PathBuf>,
        /// How many arguments we do not understand. Reported on stderr as a
        /// count, never shown (one may be a key typed in the wrong place), then
        /// Serve still runs — the npm wrapper forwards `process.argv.slice(2)` to
        /// this binary (`npm-mcp/run.js`), so exiting on an unrecognised argument
        /// would hard-break any MCP host that passes one, where the previous code
        /// ignored arguments entirely.
        ignored: usize,
    },
    /// Refused: `--docbrain-env` without a path or given twice (a file was asked
    /// for, and serving without it would run on settings nobody chose), or an
    /// option Node acts on. A fixed message, so nothing typed can be echoed.
    Invalid(&'static str),
}

const NEEDS_PATH: &str = "--docbrain-env needs a path";
const MORE_THAN_ONCE: &str = "--docbrain-env was given more than once; name one file";
const JOINED_NOT_UTF8: &str = "--docbrain-env=PATH needs a path in UTF-8; give the path as the next argument instead";
const NODE_READS_IT: &str =
    "--env-file and Node's config-file options are read by Node itself, not by docbrain-mcp; name the file with --docbrain-env";
const DOLLAR: &str = "the --docbrain-env file holds a $, which would read another variable into it; a --docbrain-env file may not (the line is not shown)";
const RELATIVE: &str = "--docbrain-env needs an absolute path: the path given is relative, and a relative path would be read from whatever directory this starts in";

/// Options Node acts on itself wherever they appear in its own argv, even after the
/// script path: it reads the named file (and runs `NODE_OPTIONS` from it) or a
/// config file before any of the npm wrapper runs. Some Node lines match them by
/// prefix, so any argument that starts with one is refused, by the wrapper and by
/// the binary alike (#603).
const NODE_READS: [&str; 4] =
    ["--env-file", "--env-file-if-exists", "--experimental-config-file", "--experimental-default-config-file"];

/// Decide what to do from the argument list (excluding argv[0]).
pub fn parse_cli(args: &[std::ffi::OsString]) -> CliAction {
    let mut docbrain_env = None;
    let mut ignored = 0;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let bytes = arg.as_encoded_bytes();
        if NODE_READS.iter().any(|f| bytes.starts_with(f.as_bytes())) {
            return CliAction::Invalid(NODE_READS_IT);
        }
        let value = match arg.to_str() {
            Some("--version" | "-V") => return CliAction::Version,
            Some("--help" | "-h") => return CliAction::Help,
            Some("--docbrain-env") => it.next().cloned().unwrap_or_default(),
            Some(other) => match other.strip_prefix("--docbrain-env=") {
                Some(v) => v.into(),
                None => {
                    ignored += 1;
                    continue;
                }
            },
            None if bytes.starts_with(b"--docbrain-env=") => return CliAction::Invalid(JOINED_NOT_UTF8),
            None => {
                ignored += 1;
                continue;
            }
        };
        if value.is_empty() {
            return CliAction::Invalid(NEEDS_PATH);
        }
        if docbrain_env.is_some() {
            return CliAction::Invalid(MORE_THAN_ONCE);
        }
        docbrain_env = Some(std::path::PathBuf::from(value));
    }
    CliAction::Serve { docbrain_env, ignored }
}

// docbrain-env:begin
/// Who may set each of DocBrain's own variables from a `--docbrain-env` file (#603, owner
/// ruling 2026-10-24: an allow-list). A named file may set a variable only where this table
/// says so; any other variable in the file, DocBrain's or not, refuses the whole file. The
/// server, the key and the profile choose who the CLI is, so the CLI takes them only from the
/// shell, its flags and its saved sign-ins; the callback host decides where the one-time login
/// code goes, so the CLI takes it only from the shell too. The connector's file exists to hold
/// its key and server. A test in each crate fails when the binary reads a `DOCBRAIN_` variable that this
/// table does not list, or when the table lets a binary's file set one it never reads.
const DOCBRAIN_VARIABLES: &[(&str, &[Binary])] = &[
    ("DOCBRAIN_API_KEY", &[Binary::Connector]),
    ("DOCBRAIN_SERVER_URL", &[Binary::Connector]),
    ("DOCBRAIN_PROFILE", &[]),
    ("DOCBRAIN_CORPUS_DROP_ALERT", &[Binary::Cli]),
    ("DOCBRAIN_GENERATE_TIMEOUT_SECS", &[Binary::Cli]),
    ("DOCBRAIN_CLI_CALLBACK_HOST", &[]),
];

/// The two binaries that read a `--docbrain-env` file.
#[derive(Clone, Copy, PartialEq)]
enum Binary {
    Cli,
    Connector,
}

/// Whether `this` binary's `--docbrain-env` file may set `name`, spelled exactly.
fn file_may_set(name: &str, this: Binary) -> bool {
    DOCBRAIN_VARIABLES.iter().any(|(n, who)| *n == name && who.contains(&this))
}

/// The variables `this` binary's file may set, as the refusal names them.
fn file_may_set_list(this: Binary) -> String {
    DOCBRAIN_VARIABLES.iter().filter(|(_, who)| who.contains(&this)).map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
}
// docbrain-env:end

/// Load the variables in the file `--docbrain-env` names, and nothing else (#603).
///
/// No `.env` is read on its own: one in the directory the connector starts in, or in any
/// parent, could set a proxy or CA variable and read the API key off the wire. Only an absolute
/// path is accepted, because a host starts the connector in whatever project is open and a
/// relative path would be read from that project. The whole file is read and every name
/// checked before any variable is set: the file may set only DocBrain's own variables the
/// connector reads (`DOCBRAIN_VARIABLES`), and anything else refuses it whole. Variables the
/// host already set to a non-empty value win over the file. No message shows the path, a value
/// or a line.
///
/// # Safety
///
/// Writes the process environment: call it only while the process has one thread (before the
/// async runtime starts), so that no other thread reads or writes the environment meanwhile.
pub unsafe fn load_docbrain_env(path: &std::path::Path) -> Result<(), String> {
    if !path.is_absolute() {
        return Err(RELATIVE.to_string());
    }
    let read_error = |e| {
        let why = match e {
            dotenvy::Error::Io(io) => io.to_string(),
            dotenvy::Error::LineParse(..) => "a line is not NAME=value (the line is not shown: it may hold a secret)".to_string(),
            _ => "it could not be loaded".to_string(),
        };
        format!("cannot read the --docbrain-env file: {why}")
    };
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read the --docbrain-env file: {e}"))?;
    // A byte-order mark (Windows editors, PowerShell) is not part of the first name.
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    // The parser replaces `$NAME` and `${NAME}` with another variable from the environment, so
    // an allowed name could carry a value the file may not set: no `$` at all.
    if text.contains(&b'$') {
        return Err(DOLLAR.to_string());
    }
    let mut vars = Vec::new();
    for item in dotenvy::from_read_iter(text) {
        vars.push(item.map_err(read_error)?);
    }
    let mut refused: Vec<&str> = vars.iter().map(|(name, _)| name.as_str()).filter(|n| !file_may_set(n, Binary::Connector)).collect();
    refused.sort_unstable();
    refused.dedup();
    if !refused.is_empty() {
        return Err(format!(
            "the --docbrain-env file sets {}; a --docbrain-env file may set only DocBrain's own variables that docbrain-mcp reads: {}",
            refused.join(", "),
            file_may_set_list(Binary::Connector)
        ));
    }
    if vars.iter().any(|(_, value)| value.contains('\0')) {
        return Err("cannot read the --docbrain-env file: a value holds a NUL byte (the value is not shown)".to_string());
    }
    for (name, value) in vars {
        // An empty value counts as unset: an editor's empty placeholder, or an image that
        // declares the variable empty, must not silently cancel the file the user named.
        if std::env::var_os(&name).is_none_or(|v| v.is_empty()) {
            // SAFETY: the caller guarantees this process has one thread (see `# Safety`).
            unsafe { std::env::set_var(name, value) };
        }
    }
    Ok(())
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
         \x20   docbrain-mcp                       run the JSON-RPC loop\n\
         \x20   docbrain-mcp --docbrain-env PATH   load DOCBRAIN_API_KEY and DOCBRAIN_SERVER_URL\n\
         \x20                                      from PATH first (an absolute path)\n\
         \x20   docbrain-mcp --version             print version\n\
         \x20   docbrain-mcp --help                print this help\n\
         \n\
         ENVIRONMENT:\n\
         \x20   DOCBRAIN_API_KEY      required to serve; create with\n\
         \x20                         `docbrain token create --name \"MCP Key\" --role viewer`\n\
         \x20   DOCBRAIN_SERVER_URL   defaults to http://localhost:3000\n\
         \x20   Set them in the MCP host's env block. No .env file is read unless\n\
         \x20   --docbrain-env names it. That file may set only DocBrain's own variables\n\
         \x20   (the two above); a non-empty value the host sets wins over it.\n",
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

/// The DocBrain MCP tools, over whichever [`Backend`] reaches the server. The routes the tools
/// call are the only authority on what a caller may do: no tool refuses on a role of its own.
pub struct McpServer<B: Backend = HttpBackend> {
    backend: B,
}

/// The protocol versions the stdio connector negotiates in `initialize`. It speaks the legacy,
/// handshake-based protocol only; a version not listed here gets the first, which is what it
/// always answered (#601 rev 8 §4.1). 2025-03-26 is not listed: it requires JSON-RPC batches, which
/// this connector does not read (slice A review R8).
const STDIO_VERSIONS: [&str; 3] = ["2024-11-05", "2025-06-18", "2025-11-25"];

/// A tool that could not do its job: the text the person reads, marked as an error so the host
/// shows it as one (a tool execution error, not a protocol error).
fn tool_error(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
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
/// `nothing_checkable`, `unavailable` and an absent `verification` produce
/// NEITHER. `nothing_checkable`: the pass ran and the answer cites nothing a
/// listing could settle. `unavailable`: the server could not read its file
/// listings, so nothing was checked. Absent: this deployment does not verify
/// (no source has a listing, or the pass is off). None of them is a verdict, and
/// reporting a verdict that was never computed is the exact failure
/// `Answer::verification` being an `Option` exists to prevent.
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

/// One markdown line per source an answer cites. A document is a link, and a
/// document with no URL is left out, as it always was. A captured note carries
/// the label the server rendered for it (who wrote it, what checks it), in the server's
/// words with no words of ours, escaped as markdown text, and is listed even with no URL: captures often have
/// none, and leaving them out hid the note the answer rests on.
fn source_lines(response: &Value) -> Vec<String> {
    let Some(arr) = response["sources"].as_array() else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|s| {
            let title = s["title"].as_str().unwrap_or("Source");
            let url = s["source_url"].as_str().unwrap_or("");
            let label = match (
                s["capture"]["attribution_line"].as_str(),
                s["capture"]["check_label"].as_str(),
            ) {
                (Some(who), Some(check)) => Some(format!("{} · {}", markdown_text(who), markdown_text(check))),
                _ => None,
            };
            match (url.is_empty(), label) {
                (true, None) => None,
                (false, None) => Some(format!("- [{title}]({url})")),
                (true, Some(label)) => Some(format!("- {title} — {label}")),
                (false, Some(label)) => Some(format!("- [{title}]({url}) — {label}")),
            }
        })
        .collect()
}

/// The server's words as markdown text: every ASCII punctuation character is backslash-escaped,
/// which CommonMark renders as the character itself. A crafted name then shows literally and can
/// never become a link, an image, emphasis, code, HTML, or an autolink (the `:`, `/`, `.` and `@`
/// of a URL or an address are escaped too).
fn markdown_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        if c.is_ascii_punctuation() {
            out.push('\\');
        }
        out.push(c);
    }
    out
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

impl McpServer<HttpBackend> {
    // `Default` would silently read process env (`DOCBRAIN_SERVER_URL`/
    // `DOCBRAIN_API_KEY`), which is surprising for a Default impl. Callers
    // should pick `new()` (env-driven) or `from_parts()` (explicit).
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        let server_url = std::env::var("DOCBRAIN_SERVER_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());
        let api_key = std::env::var("DOCBRAIN_API_KEY").ok();
        Self::from_parts(server_url, api_key)
    }

    /// Test-friendly constructor that avoids touching process env vars.
    ///
    /// Production code (`fn main`) uses `McpServer::new()` which reads
    /// `DOCBRAIN_SERVER_URL` / `DOCBRAIN_API_KEY` from the environment.
    /// Integration tests must not mutate process env in parallel runs, so
    /// they construct the server with explicit values instead.
    pub fn from_parts(server_url: String, api_key: Option<String>) -> Self {
        Self::with_backend(HttpBackend::new(server_url, api_key))
    }

    /// The server this connector talks to, for its startup messages.
    pub fn server_url(&self) -> &str {
        &self.backend.server_url
    }

    pub fn has_api_key(&self) -> bool {
        self.backend.api_key.is_some()
    }

    /// Validate API key and server connectivity at startup.
    /// Returns the display name of the authenticated identity on success.
    pub async fn validate_connection(&self) -> Result<String, String> {
        let request = ApiRequest::get(ApiPath::fixed("/api/v1/auth/me"));
        match self.backend.send(request).await {
            Ok(resp) if (200..300).contains(&resp.status) => {
                let identity: Value = serde_json::from_str(&resp.body)
                    .map_err(|e| format!("Invalid /auth/me response: {}", e))?;
                let name = identity["name"]
                    .as_str()
                    .or_else(|| identity["email"].as_str())
                    .unwrap_or("(unknown)")
                    .to_string();
                let role = identity["role"].as_str().unwrap_or("unknown");
                Ok(format!("{} ({})", name, role))
            }
            Ok(resp) if resp.status == 401 => {
                Err("Invalid or expired DOCBRAIN_API_KEY (401 Unauthorized). \
                     Run: docbrain token create --name \"MCP Key\" --role viewer"
                    .to_string())
            }
            Ok(resp) => Err(format!("Server returned unexpected status: {}", resp.status)),
            Err(e) if e.starts_with(backend::CANNOT_CONNECT) => Err(format!(
                "Cannot connect to DocBrain server at {}. \
                     Check DOCBRAIN_SERVER_URL.",
                self.backend.server_url
            )),
            Err(e) => Err(format!("Connection error: {}", e)),
        }
    }
}

impl<B: Backend> McpServer<B> {
    pub fn with_backend(backend: B) -> Self {
        Self { backend }
    }

    /// The tools, as `tools/list` answers them (deterministic order).
    pub fn tools_list(&self) -> Value {
        self.handle_tools_list()
    }

    /// Run one tool. `Err` only for an unknown tool name (a protocol error); any other failure is
    /// the tool's own result, marked `isError`.
    pub async fn call_tool(&self, name: &str, arguments: &Value) -> Result<Value, JsonRpcError> {
        self.handle_tools_call(&json!({ "name": name, "arguments": arguments })).await
    }

    pub async fn handle_request(&self, req: JsonRpcRequest) -> JsonRpcResponse {
        let id = req.id.clone().unwrap_or(Value::Null);

        let result = match req.method.as_str() {
            "initialize" => Ok(self.handle_initialize(&req.params)),
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

    /// The legacy handshake: the requested version when this connector speaks it, else the
    /// version it has always answered.
    fn handle_initialize(&self, params: &Value) -> Value {
        let requested = params["protocolVersion"].as_str().unwrap_or_default();
        let version = STDIO_VERSIONS.iter().find(|v| **v == requested).copied().unwrap_or(STDIO_VERSIONS[0]);
        json!({
            "protocolVersion": version,
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
                    "description": "Ask a question about your organization's internal documentation. Uses hybrid semantic + keyword search across Confluence, runbooks, and captured Slack/GitHub threads. Preserves specific service names, team names, and tool names for precise retrieval. Corrects false premises (e.g. if you assume every engineer can deploy to production, it will correct you). Returns a direct, human-like answer — never says 'according to the documentation'. Cites up to 5 sources: documents with their URLs, and captures with who recorded them and how checked they are. For questions outside the knowledge base (HR policies, general engineering topics not documented internally), clearly says so rather than guessing. Uses session memory to resolve follow-up questions in context.",
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
                    "description": with_secret_guardrail("Edit or withdraw a capture you recorded. Use the id from docbrain_context or from the capture's report. Edit replaces its text and/or its premise set (the same premises as docbrain_annotate) and re-indexes it; withdraw takes it out of answers at once and needs a reason, which is shown with it. Only its author can edit; DocBrain-written summaries can only be withdrawn. A capture that has reached someone other than you (an answer to their question, their context lookup, a draft, or their opening an answer page that shows it) cannot be deleted, only withdrawn; until then, Delete removes it for good."),
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

        // No role check here: the route each tool calls decides, for every door alike, and its
        // refusal comes back as this tool's error. A failed tool is a tool execution error
        // (`isError`), which the host shows to the model; only an unknown tool name is a
        // protocol error.
        let outcome = match tool_name {
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
            _ => {
                return Err(JsonRpcError {
                    code: -32602,
                    message: format!("Unknown tool: {}", tool_name),
                });
            }
        };
        Ok(outcome.unwrap_or_else(|text| tool_error(&text)))
    }

    /// One request, answered as JSON; any failure is the text a tool reports.
    async fn json(&self, request: ApiRequest) -> Result<Value, ApiFailure> {
        let response = self.backend.send(request).await.map_err(|message| ApiFailure {
            status: None,
            body: String::new(),
            message,
        })?;
        if !(200..300).contains(&response.status) {
            return Err(ApiFailure {
                status: Some(response.status),
                message: format!("API error ({}): {}", response.status, response.body),
                body: response.body,
            });
        }
        serde_json::from_str(&response.body).map_err(|e| ApiFailure {
            status: None,
            body: String::new(),
            message: format!("Invalid response: {}", e),
        })
    }

    async fn tool_ask(&self, args: &Value) -> Result<Value, String> {
        let question = args["question"].as_str().ok_or_else(|| String::from("Missing 'question' parameter"))?;
        let session_id = args["session_id"].as_str();

        let mut body = json!({ "question": question });
        if let Some(sid) = session_id {
            body["session_id"] = json!(sid);
        }
        let request = ApiRequest::post(ApiPath::fixed("/api/v1/ask"), body).long();
        let response = match self.json(request).await {
            Ok(v) => v,
            Err(f) if session_id.is_some() && f.names_a_dead_session() => {
                return Err(START_NEW_CONVERSATION.to_string());
            }
            Err(f) => return Err(f.message),
        };

        let answer = response["answer"].as_str().unwrap_or("No answer available");
        let sources = source_lines(&response);

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

    async fn tool_incident(&self, args: &Value) -> Result<Value, String> {
        let description = args["description"].as_str().ok_or_else(|| String::from("Missing 'description' parameter"))?;

        let body = json!({ "description": description });
        let response = self.api_call("/api/v1/incident", &body).await?;


        let answer = response["answer"]
            .as_str()
            .unwrap_or("No incident response available");
        let sources = source_lines(&response);

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

    async fn tool_freshness(&self, args: &Value) -> Result<Value, String> {
        let space = args["space"].as_str();

        // The space is a query value; the backend encodes it.
        let mut request = ApiRequest::get(ApiPath::fixed("/api/v1/freshness"));
        if let Some(s) = space {
            request = request.query("space", s);
        }
        let data = self.json(request).await.map_err(|f| f.message)?;

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

    async fn tool_autopilot_gaps(&self, args: &Value) -> Result<Value, String> {
        // Clamp to 1..=200 (default 10): a negative/zero limit produces a
        // nonsensical query and an unbounded value would pull the whole gap
        // table in one call from a hallucinated arg.
        let limit = args["limit"].as_i64().unwrap_or(10).clamp(1, 200);

        let request = ApiRequest::get(ApiPath::fixed("/api/v1/autopilot/gaps")).query("limit", limit.to_string());
        let gaps: Vec<Value> = serde_json::from_value(self.json(request).await.map_err(|f| f.message)?)
            .map_err(|e| format!("Invalid response: {}", e))?;

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

    async fn tool_autopilot_generate(&self, args: &Value) -> Result<Value, String> {
        let cluster_id = args["cluster_id"].as_str().ok_or_else(|| String::from("Missing 'cluster_id' parameter. Get IDs from docbrain_autopilot_gaps."))?;

        let path = ApiPath::with_id("/api/v1/autopilot/generate/", cluster_id, "", "cluster_id")?;
        let result = self.json(ApiRequest::post(path, json!({}))).await.map_err(|f| f.message)?;

        // Now fetch the full draft to get the content
        let draft_id = result["draft_id"].as_str().unwrap_or("");
        let title = result["title"].as_str().unwrap_or("Untitled");
        let content_type = result["content_type"].as_str().unwrap_or("unknown");
        let quality = result["quality_score"].as_f64().unwrap_or(0.0);

        let mut text = format!(
            "Draft Generated: {} (type: {}, quality: {:.2})\nDraft ID: {}\n\n",
            title, content_type, quality, draft_id
        );

        // Fetch the draft content. The id came from the server, and still goes into a path only
        // as a checked id.
        if let Ok(path) = ApiPath::with_id("/api/v1/autopilot/drafts/", draft_id, "", "draft_id")
            && let Ok(draft) = self.json(ApiRequest::get(path)).await
            && let Some(content) = draft["content"].as_str()
        {
            text.push_str("--- Draft Content ---\n\n");
            text.push_str(content);
        }

        Ok(json!({
            "content": [{
                "type": "text",
                "text": text
            }]
        }))
    }

    async fn tool_autopilot_summary(&self, _args: &Value) -> Result<Value, String> {
        let request = ApiRequest::get(ApiPath::fixed("/api/v1/autopilot/summary"));
        let data = self.json(request).await.map_err(|f| f.message)?;

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
    async fn tool_annotate(&self, args: &Value) -> Result<Value, String> {
        let gap_id = gap_cluster_id(args)?;
        let file_path = args["file_path"].as_str().ok_or_else(|| String::from("Missing 'file_path' parameter"))?;
        let annotation = args["annotation"].as_str().ok_or_else(|| String::from("Missing 'annotation' parameter"))?;

        // Input validation
        if file_path.trim().is_empty() {
            return Err(String::from("file_path must not be empty"));
        }
        if annotation.trim().is_empty() {
            return Err(String::from("annotation must not be empty"));
        }
        if annotation.chars().count() > 10_000 {
            return Err(String::from("annotation exceeds 10000 character limit"));
        }
        if file_path.chars().count() > 500 {
            return Err(String::from("file_path exceeds 500 character limit"));
        }

        let line_range = args["line_range"].as_str();
        let fragment_type = args["fragment_type"].as_str().unwrap_or("context");
        let space = args["space"].as_str();

        // Validate fragment_type
        let valid_types = ["decision", "fact", "caveat", "procedure", "context"];
        if !valid_types.contains(&fragment_type) {
            return Err(format!(
                    "Invalid fragment_type '{}'. Must be one of: {}",
                    fragment_type,
                    valid_types.join(", ")
                ));
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
            // rev 8 §4: a route refusal (role, secret gate, duplicate) is a tool error.
            Err(msg) => Ok(tool_error(&msg)),
        }
    }

    /// `docbrain_suggest_capture` — surface unanswered questions about a code area.
    ///
    /// Queries autopilot gaps and filters for relevance to the given file/function.
    async fn tool_suggest_capture(&self, args: &Value) -> Result<Value, String> {
        let file_path = args["file_path"].as_str().ok_or_else(|| String::from("Missing 'file_path' parameter"))?;
        let function_name = args["function_name"].as_str();

        // Fetch gap clusters from autopilot — get more than needed, filter client-side
        let request = ApiRequest::get(ApiPath::fixed("/api/v1/autopilot/gaps")).query("limit", "50");
        let gaps: Vec<Value> = serde_json::from_value(self.json(request).await.map_err(|f| f.message)?)
            .map_err(|e| format!("Invalid response: {}", e))?;

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
    async fn tool_context(&self, args: &Value) -> Result<Value, String> {
        let paths: Vec<String> = args["file_paths"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        if paths.is_empty() {
            return Err(String::from("Missing or empty 'file_paths' parameter"));
        }

        let request = ApiRequest::post(ApiPath::fixed("/api/v1/fragments/context"), json!({ "file_paths": paths }));
        let body = self.json(request).await.map_err(|f| f.message)?;

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
    async fn tool_commit_capture(&self, args: &Value) -> Result<Value, String> {
        let gap_id = gap_cluster_id(args)?;
        let intent = args["intent"].as_str().ok_or_else(|| String::from("Missing 'intent' parameter — why was this change made?"))?;

        // Input validation
        if intent.trim().is_empty() {
            return Err(String::from("intent must not be empty"));
        }
        if intent.chars().count() > 10_000 {
            return Err(String::from("intent exceeds 10000 character limit"));
        }
        let diff_summary = args["diff_summary"].as_str().unwrap_or("");
        if diff_summary.chars().count() > 5_000 {
            return Err(String::from("diff_summary exceeds 5000 character limit"));
        }
        let commit_message = args["commit_message"].as_str().unwrap_or("");
        if commit_message.chars().count() > 1_000 {
            return Err(String::from("commit_message exceeds 1000 character limit"));
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
            // rev 8 §4: a route refusal (role, secret gate, duplicate) is a tool error.
            Err(msg) => Ok(tool_error(&msg)),
        }
    }

    async fn tool_feedback(&self, args: &Value) -> Result<Value, String> {
        let episode_id = args["episode_id"].as_str().ok_or_else(|| String::from("Missing 'episode_id' parameter"))?;
        let thumbs_up = args["thumbs_up"].as_bool().ok_or_else(|| String::from("Missing 'thumbs_up' parameter (true or false)"))?;

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
    async fn create_fragment(&self, body: &Value) -> Result<Result<Value, String>, String> {
        let request = ApiRequest::post(ApiPath::fixed("/api/v1/fragments"), body.clone());
        Ok(self.api_request(request).await?.map_err(|f| describe_failure(&f)))
    }

    /// One JSON request to the DocBrain API. The outer `Err` is transport (server unreachable,
    /// unreadable answer); the inner `Err` is the server's refusal, parsed.
    async fn api_request(&self, request: ApiRequest) -> Result<Result<Value, CaptureFailure>, String> {
        let response = self.backend.send(request).await?;
        if (200..300).contains(&response.status) {
            if response.body.trim().is_empty() {
                return Ok(Ok(Value::Null));
            }
            let v = serde_json::from_str(&response.body).map_err(|e| format!("Invalid response: {}", e))?;
            Ok(Ok(v))
        } else {
            Ok(Err(parse_failure(response.status, &response.body)))
        }
    }

    async fn tool_revise(&self, args: &Value) -> Result<Value, String> {
        let id = args
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| String::from("id is required"))?;
        let intent = revise_intent(args)?;
        // The id goes into a URL path: only a UUID-shaped id may.
        let (method, path, body, done) = match &intent {
            ReviseIntent::Edit(b) => (
                Method::PATCH,
                ApiPath::with_id("/api/v1/fragments/", id, "", "id").map_err(|_| "id must be a capture id")?,
                b.clone(),
                "Capture edited and re-indexed",
            ),
            ReviseIntent::Withdraw(r) => (
                Method::POST,
                ApiPath::with_id("/api/v1/fragments/", id, "/withdraw", "id").map_err(|_| "id must be a capture id")?,
                json!({"reason": r}),
                "Capture withdrawn: it is no longer used in answers",
            ),
        };
        let text = match self.api_request(ApiRequest::send_json(method, path, body)).await? {
            Ok(v) => {
                let draft = v
                    .get("carried_by_draft")
                    .and_then(Value::as_str)
                    .map(|d| {
                        format!("\nA composed draft ({d}) still carries its text; withdraw does not change the draft.")
                    })
                    .unwrap_or_default();
                format!("{done}. id {id}{draft}{}", warning_lines(&v))
            }
            Err(f) => return Ok(tool_error(&describe_revise_failure(&f))),
        };
        Ok(json!({"content": [{"type": "text", "text": text}]}))
    }

    /// POST `body` to a fixed API path and read the JSON answer.
    pub async fn api_call(&self, path: &'static str, body: &Value) -> Result<Value, String> {
        self.json(ApiRequest::post(ApiPath::fixed(path), body.clone())).await.map_err(|f| f.message)
    }
}

/// `docbrain_ask`'s `session_id` description, shared with the docs.
const SESSION_ID_DESCRIPTION: &str = "Continue a conversation: pass only a session_id returned by a previous docbrain_ask, so a follow-up is understood in context. Omit it to start a new conversation. A conversation can be continued by the same DocBrain user (the API key's linked user, or the key itself when it has none).";

/// The tool error for a `session_id` the server will not continue.
const START_NEW_CONVERSATION: &str = "start a new conversation: omit session_id";

/// Appended to every capture tool and to each argument whose text becomes the capture, so the
/// model leaves secrets out before anything leaves the machine. The server's refusal is the
/// backstop; this is where the model reads what to write.
const CAPTURE_SECRET_GUARDRAIL: &str = "Never include a secret value: a password, token, API key, private key, or a connection string or URL with credentials. Write where it lives instead, e.g. 'the database password is in the team vault' or the variable that carries it, DB_PASSWORD. DocBrain refuses a capture that holds a secret.";

/// A capture tool's description, or a text argument's, followed by the secret guardrail.
fn with_secret_guardrail(description: &str) -> String {
    format!("{description} {CAPTURE_SECRET_GUARDRAIL}")
}

/// What the model is told when the server refuses a capture for a secret. It names no value:
/// the tool never repeats the text it sent. #599 ruling 6 (owner 2026-10-24): the person decides,
/// never the agent: report the refusal exactly as given, ask whether that line holds a secret, and
/// never edit or resend on its own.
const SECRET_REFUSAL_GUIDANCE: &str = "Report this refusal to the person exactly as given, and ask them whether that line holds a secret. Never edit, reword, split, encode or resend the text on your own: the person decides what to change.";

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



/// The optional `gap_id` argument, sent as `gap_cluster_id`. Absent is fine; present
/// must be a UUID-shaped string.
fn gap_cluster_id(args: &Value) -> Result<Option<String>, String> {
    match args.get("gap_id") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(g)) if is_uuid_shaped(g) => Ok(Some(g.clone())),
        Some(_) => Err("gap_id must be an id".to_string()),
    }
}

/// `Fragment ID: .. | Status: ..` plus one `Warning: ` line per server warning.
fn capture_report(result: &Value) -> String {
    let id = result["id"].as_str().unwrap_or("unknown");
    let status = result["status"].as_str().unwrap_or("unknown");
    format!("Fragment ID: {id} | Status: {status}{}", warning_lines(result))
}

/// One `\nWarning: ` line per server warning, in the server's words (a create's and an edit's).
fn warning_lines(result: &Value) -> String {
    result["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|w| format!("\nWarning: {w}"))
        .collect()
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
/// What an agent does when a declared premise names a file no connected repository has any more
/// (#457 spec point 18): one action, never a blind resend.
const PREMISE_REMOVED_GUIDANCE: &str = "Read the repository's file tree, correct that premise or leave it out, and show the corrected draft for a new yes; do not resend it unchanged.";
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
        // A resend of a write that finished after its client left is the common case (#601 rev 8
        // §4.2, B6): the capture is there, so the line says so first.
        "already_captured" => format!(
            "Already saved: {}. DocBrain already has this capture; use docbrain_revise to change it.",
            f.id.as_deref().unwrap_or("id not given")
        ),
        // Another capture at the same anchor: its id, so the person can be offered a revise (rev 8
        // §4.2, B6). Not "Already saved": the text sent may differ from the one already there.
        "duplicate_source" if f.id.is_some() => format!(
            "{verb} ({}): {} (id {}); use docbrain_revise on it to change that capture.",
            f.status,
            f.message,
            f.id.as_deref().unwrap_or_default()
        ),
        // Any secret refusal kind: the server's words, then what to do. Only the server's
        // message is relayed, never the text that was sent.
        kind if kind.starts_with("secret_") => {
            format!("{verb} ({}): {}\n{SECRET_REFUSAL_GUIDANCE}", f.status, f.message)
        }
        // One retry, then stop, so an agent never loops into the cooldown.
        "not_checked" => format!("{NOT_CHECKED_GUIDANCE} ({}: {})", f.status, f.message),
        "capture_changed" => format!("{CAPTURE_CHANGED_GUIDANCE} ({}: {})", f.status, f.message),
        "premise_removed" => format!("{verb} ({}): {}\n{PREMISE_REMOVED_GUIDANCE}", f.status, f.message),
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

    fn args(a: &[&str]) -> Vec<std::ffi::OsString> {
        a.iter().map(|s| s.into()).collect()
    }

    fn serve(docbrain_env: Option<&str>, ignored: usize) -> CliAction {
        CliAction::Serve { docbrain_env: docbrain_env.map(std::path::PathBuf::from), ignored }
    }

    #[test]
    fn version_flag_is_recognised() {
        assert_eq!(parse_cli(&args(&["--version"])), CliAction::Version);
        assert_eq!(parse_cli(&args(&["-V"])), CliAction::Version);
    }

    #[test]
    fn help_flag_is_recognised() {
        assert_eq!(parse_cli(&args(&["--help"])), CliAction::Help);
        assert_eq!(parse_cli(&args(&["-h"])), CliAction::Help);
    }

    #[test]
    fn no_arguments_serves_the_protocol() {
        assert_eq!(parse_cli(&[]), serve(None, 0));
    }

    /// An unrecognised argument is counted so the binary can warn about it,
    /// and never kept (one may be a key). BLINDSPOT (pre-push): it must NOT
    /// cause an exit — `npm-mcp/run.js` forwards `process.argv.slice(2)`, so
    /// exiting would break any MCP host that passes an argument, which worked
    /// before argument handling existed. See the dispatch in `main.rs`.
    #[test]
    fn unknown_arguments_are_counted() {
        assert_eq!(parse_cli(&args(&["--frobnicate"])), serve(None, 1));
        assert_eq!(parse_cli(&args(&["serve"])), serve(None, 1));
        assert_eq!(parse_cli(&args(&["--api-key", "SYNTH_PASTED_KEY"])), serve(None, 2));
    }

    // #603: `--docbrain-env` is the only way an env file is read.

    #[test]
    fn docbrain_env_takes_the_next_argument_or_the_joined_value() {
        assert_eq!(parse_cli(&args(&["--docbrain-env", "/etc/db.env"])), serve(Some("/etc/db.env"), 0));
        assert_eq!(parse_cli(&args(&["--docbrain-env=/etc/db.env"])), serve(Some("/etc/db.env"), 0));
        assert_eq!(parse_cli(&args(&["--frobnicate", "--docbrain-env", "/etc/db.env", "serve"])), serve(Some("/etc/db.env"), 2));
    }

    #[test]
    fn docbrain_env_without_a_path_is_refused() {
        for a in [&["--docbrain-env"][..], &["--docbrain-env="][..], &["--docbrain-env", ""][..]] {
            assert_eq!(parse_cli(&args(a)), CliAction::Invalid(NEEDS_PATH), "{a:?}");
        }
    }

    #[test]
    fn docbrain_env_twice_is_refused_even_with_the_same_path() {
        for a in [&["--docbrain-env", "/a.env", "--docbrain-env", "/b.env"][..], &["--docbrain-env=/a.env", "--docbrain-env=/a.env"][..]] {
            assert_eq!(parse_cli(&args(a)), CliAction::Invalid(MORE_THAN_ONCE), "{a:?}");
        }
    }

    /// Node acts on these itself; the binary refuses them wherever they are,
    /// joined or not, even where Node would have taken them as a value.
    #[test]
    fn options_node_acts_on_are_refused() {
        for f in NODE_READS {
            for a in [vec![f.to_string(), "/x.env".into()], vec![format!("{f}=/x.env")], vec!["--frobnicate".into(), format!("{f}=/x")]] {
                let a: Vec<&str> = a.iter().map(String::as_str).collect();
                assert_eq!(parse_cli(&args(&a)), CliAction::Invalid(NODE_READS_IT), "{a:?}");
            }
        }
        // Some Node lines match these by prefix, so a longer name is refused too.
        for odd in ["--env-files=/x", "--env-filex", "--experimental-default-config-filex", "--experimental-config-file-x"] {
            assert_eq!(parse_cli(&args(&[odd])), CliAction::Invalid(NODE_READS_IT), "{odd}");
        }
    }

    /// Asking what the binary is never needs the file, wherever the flag sits.
    #[test]
    fn version_and_help_need_no_docbrain_env_file() {
        assert_eq!(parse_cli(&args(&["--docbrain-env", "/missing.env", "--version"])), CliAction::Version);
        assert_eq!(parse_cli(&args(&["--frobnicate", "-h"])), CliAction::Help);
    }

    #[test]
    fn a_relative_docbrain_env_file_is_refused_before_it_is_opened() {
        for p in ["db.env", "./db.env", "~/db.env", ""] {
            // SAFETY: a relative path is refused before the environment is touched.
            let err = unsafe { load_docbrain_env(std::path::Path::new(p)) }.unwrap_err();
            assert_eq!(err, RELATIVE, "{p:?}");
        }
    }

    #[test]
    fn the_connector_s_file_may_set_only_its_key_and_server() {
        assert!(file_may_set("DOCBRAIN_API_KEY", Binary::Connector));
        assert!(file_may_set("DOCBRAIN_SERVER_URL", Binary::Connector));
        for no in ["docbrain_api_key", "DOCBRAIN_PROFILE", "DOCBRAIN_CORPUS_DROP_ALERT", "XDG_CONFIG_HOME", "HTTPS_PROXY", "RUST_LOG", ""] {
            assert!(!file_may_set(no, Binary::Connector), "{no}");
        }
        assert_eq!(file_may_set_list(Binary::Connector), "DOCBRAIN_API_KEY, DOCBRAIN_SERVER_URL");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server() -> McpServer {
        McpServer::from_parts("http://localhost:9999".to_string(), Some("test_key".to_string()))
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
            err.contains("file_path"),
            "error should mention file_path: {}",
            err
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
            err.contains("annotation"),
            "error should mention annotation: {}",
            err
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
            err.contains("10000"),
            "error should mention limit: {}",
            err
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
        assert!(err.contains("Invalid fragment_type"));
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
        assert!(err.contains("intent"));
    }

    #[tokio::test]
    async fn commit_capture_validates_intent_length() {
        let server = make_server();
        let long = "x".repeat(10_001);
        let args = json!({ "intent": long });
        let result = server.tool_commit_capture(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("10000"));
    }

    #[tokio::test]
    async fn commit_capture_validates_diff_summary_length() {
        let server = make_server();
        let long = "x".repeat(5_001);
        let args = json!({ "intent": "fix bug", "diff_summary": long });
        let result = server.tool_commit_capture(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("5000"));
    }

    #[tokio::test]
    async fn suggest_capture_validates_missing_file_path() {
        let server = make_server();
        let args = json!({});
        let result = server.tool_suggest_capture(&args).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("file_path"));
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
        assert!(!err.starts_with("API call failed") && !err.starts_with(backend::CANNOT_CONNECT), "invalid params, not a transport failure: {err}");
        assert!(err.contains("file_paths"));
    }

    #[tokio::test]
    async fn context_rejects_empty_file_paths() {
        let server = make_server();
        let args = json!({ "file_paths": [] });
        let result = server.tool_context(&args).await;
        let err = result.unwrap_err();
        assert!(!err.starts_with("API call failed") && !err.starts_with(backend::CANNOT_CONNECT), "invalid params, not a transport failure: {err}");
        assert!(err.contains("empty"));
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
        let init = server.handle_initialize(&json!({}));
        assert_eq!(init["protocolVersion"], "2024-11-05", "no version asked: what stdio always answered");
        assert!(init["capabilities"]["tools"].is_object());
        assert_eq!(init["serverInfo"]["name"], "docbrain");
        // #601 rev 8 §4.1: a legacy version this connector speaks is echoed; any other gets 2024-11-05.
        for v in ["2024-11-05", "2025-06-18", "2025-11-25"] {
            assert_eq!(server.handle_initialize(&json!({"protocolVersion": v}))["protocolVersion"], v);
        }
        // 2025-03-26 requires batches, which stdio does not read.
        for other in ["2025-03-26", "2026-07-28", "1900-01-01", ""] {
            assert_eq!(server.handle_initialize(&json!({"protocolVersion": other}))["protocolVersion"], "2024-11-05", "{other}");
        }
    }

    #[tokio::test]
    async fn annotate_rejects_empty_file_path() {
        let server = make_server();
        let args = json!({ "file_path": "  ", "annotation": "some knowledge" });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[tokio::test]
    async fn annotate_rejects_empty_annotation() {
        let server = make_server();
        let args = json!({ "file_path": "src/main.rs", "annotation": "" });
        let result = server.tool_annotate(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    #[tokio::test]
    async fn commit_capture_rejects_empty_intent() {
        let server = make_server();
        let args = json!({ "intent": "   " });
        let result = server.tool_commit_capture(&args).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
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
        assert!(describe_failure(&dup).starts_with("Already saved: abc."), "{}", describe_failure(&dup));
    }

    #[test]
    fn a_removed_premise_is_reported_with_one_action() {
        let body = r#"{"error":"not saved: no connected repository has src/gone.rs any more (one had it before); check the path, or leave it out of the premises, and send again","kind":"premise_removed","paths":["src/gone.rs"]}"#;
        let f = parse_failure(422, body);
        let text = describe_failure(&f);
        assert!(text.contains("(422): not saved: no connected repository has src/gone.rs any more"), "{text}");
        assert!(text.ends_with(PREMISE_REMOVED_GUIDANCE), "{text}");
        assert!(PREMISE_REMOVED_GUIDANCE.contains("do not resend it unchanged"));
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
            assert_eq!(e, "gap_id must be an id");
        }
        assert!(gap_cluster_id(&serde_json::json!({"gap_id": 5})).is_err(), "not a string");
    }

    #[test]
    fn an_edit_reports_the_servers_warnings_verbatim() {
        let v = serde_json::json!({"id": "i1", "status": "indexed", "warnings": ["the file this capture rests on does not exist: a/b.md"]});
        assert_eq!(warning_lines(&v), "\nWarning: the file this capture rests on does not exist: a/b.md");
        assert_eq!(warning_lines(&serde_json::json!({"id": "i1"})), "");
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
                // #599 ruling 6: report and ask; never edit or resend on its own.
                assert!(t.contains("ask them whether that line holds a secret"), "{t}");
                assert!(t.contains("Never edit, reword, split, encode or resend the text on your own"), "{t}");
                assert!(!t.contains("Remove the value"), "the old guidance is gone: {t}");
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
