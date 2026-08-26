# DocBrain client tooling (source)

This directory contains the full source for everything DocBrain runs **on your side of the network boundary**:

| Crate | What it is | License |
|---|---|---|
| [`docbrain-cli`](docbrain-cli/) | The `docbrain` command-line client: `login`, `ask`, `capture`, `generate`, `freshness`, `evidence` — and the standalone `docbrain-verify` binary | MIT |
| [`docbrain-mcp`](docbrain-mcp/) | The MCP server installed in Claude Code, Cursor, and other MCP-compatible editors | MIT |
| [`docbrain-evidence`](docbrain-evidence/) | The offline verifier for `.dbev` evidence bundles — audit a signed record of your knowledge without trusting DocBrain | MIT |

## Why these are open

The code most worth auditing is the code that runs in *your* environment, holds *your* credentials, and decides *what leaves your machine*. That's these crates. Read them, build them, diff them against the released binaries.

The DocBrain server ships as free production Docker images under BSL 1.1; its source is not published. See the [project status note](../README.md) for the honest state of that.

## Building

```bash
cargo build --workspace   # from the repo root
cargo test --workspace
```

Built and tested in public CI on every change: see the `clients` workflow.

## Syncing

Development happens in the private monorepo; these crates are synced here verbatim on every release. File issues and PRs against this repo — accepted changes are ported back upstream and ship in the next release.
