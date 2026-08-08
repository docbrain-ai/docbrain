//! DocBrain MCP server binary entry point.
//!
//! All server logic lives in `docbrain_mcp` (lib target). This binary is a
//! thin shim that:
//!   1. Loads `.env`.
//!   2. Validates the API key + connectivity at startup.
//!   3. Drives the JSON-RPC stdin/stdout loop.
//!
//! The split (lib + bin) exists so integration tests can drive `McpServer`
//! directly without subprocess gymnastics. See
//! `tests/header_caller_integration.rs`.

use anyhow::Result;
use docbrain_mcp::{JsonRpcRequest, McpServer};
use serde_json::json;
use std::io::{self, BufRead, Write};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let server = McpServer::new();

    // --- Startup validation ---
    if server.api_key.is_none() {
        eprintln!("[docbrain-mcp] ERROR: DOCBRAIN_API_KEY is not set.");
        eprintln!(
            "[docbrain-mcp] This binary is the editor connector, not the product — it needs a"
        );
        eprintln!(
            "[docbrain-mcp] self-hosted DocBrain server to talk to. Deploy one in ~5 minutes:"
        );
        eprintln!("[docbrain-mcp]   https://github.com/docbrain-ai/docbrain#quickstart");
        eprintln!(
            "[docbrain-mcp] Create a key with: docbrain token create --name \"MCP Key\" --role viewer"
        );
        eprintln!("[docbrain-mcp] Then add DOCBRAIN_API_KEY to your MCP config env block.");
        // MCP servers communicate over stdin/stdout. eprintln is intentional here for
        // user-visible diagnostics on stderr — tracing would also go to stderr but with
        // extra formatting that may confuse MCP hosts.
        std::process::exit(1);
    }
    match server.validate_connection().await {
        Ok(identity) => {
            eprintln!(
                "[docbrain-mcp] Connected to {} as {}",
                server.server_url, identity
            );
            // MCP diagnostics: eprintln is intentional (stdout reserved for JSON-RPC)
        }
        Err(msg) => {
            eprintln!("[docbrain-mcp] ERROR: {}", msg);
            eprintln!(
                "[docbrain-mcp] Is a DocBrain server running at {}? This connector requires one:",
                server.server_url
            );
            eprintln!("[docbrain-mcp]   https://github.com/docbrain-ai/docbrain#quickstart");
            std::process::exit(1);
        }
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let error_response = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("Parse error: {}", e) }
                });
                writeln!(stdout, "{}", serde_json::to_string(&error_response)?)?;
                stdout.flush()?;
                continue;
            }
        };

        // JSON-RPC notifications (no id) must not receive a response
        let is_notification = request.id.is_none();

        let response = server.handle_request(request).await;

        if !is_notification {
            writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
            stdout.flush()?;
        }
    }

    Ok(())
}
