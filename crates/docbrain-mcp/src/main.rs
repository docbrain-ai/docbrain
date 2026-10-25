// SPDX-License-Identifier: MIT
//! DocBrain MCP server binary entry point.
//!
//! All server logic lives in `docbrain_mcp` (lib target). This binary is a
//! thin shim that:
//!   1. Reads its arguments, and loads the file `--docbrain-env` names, if any.
//!      No `.env` is ever read on its own (#603).
//!   2. Validates the API key + connectivity at startup.
//!   3. Drives the JSON-RPC stdin/stdout loop.
//!
//! The split (lib + bin) exists so integration tests can drive `McpServer`
//! directly without subprocess gymnastics. See
//! `tests/header_caller_integration.rs`.

use anyhow::{Context, Result};
use docbrain_mcp::{load_docbrain_env, parse_cli, usage, CliAction, JsonRpcRequest, McpServer};
use serde_json::json;
use std::io::{self, BufRead, Write};

/// Arguments and the `--docbrain-env` file are handled here, before the async runtime starts:
/// loading the file writes the process environment, which is only sound while the process has
/// one thread.
fn main() -> Result<()> {
    // Arguments first. `--version` and `--help` are questions about this
    // binary, so they must be answerable without credentials — startup
    // validation used to run first, and asking for the version exited 1 with
    // "DOCBRAIN_API_KEY is not set". `args_os`: an argument that is not UTF-8
    // is counted like any other, never a panic.
    let docbrain_env = match parse_cli(&std::env::args_os().skip(1).collect::<Vec<_>>()) {
        CliAction::Version => {
            println!("docbrain-mcp {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        CliAction::Help => {
            print!("{}", usage());
            return Ok(());
        }
        CliAction::Invalid(msg) => {
            eprintln!("[docbrain-mcp] ERROR: {msg}");
            eprint!("{}", usage());
            std::process::exit(2);
        }
        CliAction::Serve { docbrain_env, ignored } => {
            // Warn, do NOT exit. The npm wrapper forwards argv to this binary,
            // so exiting here would break an MCP host that passes an argument —
            // a case that worked before argument handling existed at all. The
            // arguments are counted, never shown: one may be a key typed in the
            // wrong place, and hosts write stderr to their logs.
            if ignored > 0 {
                eprintln!(
                    "[docbrain-mcp] WARNING: ignoring {ignored} unrecognised argument(s); they are not shown, in case one holds a secret"
                );
                eprint!("{}", usage());
            }
            docbrain_env
        }
    };

    // Before anything reads the environment: `McpServer::new()` reads the server
    // URL and key, and the HTTP clients it builds read the proxy variables.
    if let Some(path) = docbrain_env {
        // SAFETY: the async runtime has not started; this process has one thread.
        if let Err(msg) = unsafe { load_docbrain_env(&path) } {
            eprintln!("[docbrain-mcp] ERROR: {msg}");
            std::process::exit(1);
        }
    }

    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("start the async runtime")?
        .block_on(serve())
}

async fn serve() -> Result<()> {
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
        eprintln!(
            "[docbrain-mcp] No .env file is read: to keep the key in a file, pass --docbrain-env with the file's full path."
        );
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
