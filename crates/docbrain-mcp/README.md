# docbrain-mcp

MCP server that connects Claude Code, Cursor, and any MCP-compatible editor to your organization's [DocBrain](https://github.com/docbrain-ai/docbrain) instance.

**This is the connector, not the product.** It's a thin stdio adapter that translates MCP tool calls into REST calls against a DocBrain server — all data and logic live server-side. Without a running DocBrain server it does nothing (and will tell you so at startup).

```
Your editor (agent) ──stdio──> docbrain-mcp ──HTTPS──> your DocBrain server
```

## Prerequisites

A self-hosted DocBrain server — deploy one in ~5 minutes: [Quickstart](https://github.com/docbrain-ai/docbrain#quickstart)

## Setup

```bash
claude mcp add docbrain \
  --env DOCBRAIN_API_KEY=db_sk_... \
  --env DOCBRAIN_SERVER_URL=https://docbrain.your-org.internal \
  -- docbrain-mcp
```

Create a scoped key with `docbrain token create --name "MCP Key" --role viewer` (use a key with capture permission if you want the write tools below).

## Tools

| Tool | Direction | What it does |
|------|-----------|--------------|
| `docbrain_ask` | read | Cited answers from your org's memory, in the editor |
| `docbrain_suggest_capture` | read | Checks for documentation gaps around a file or function |
| `docbrain_capture` | write | Files a fix/decision/caveat as a fragment into the review queue |
| `docbrain_commit_capture` | write | Captures the *why* behind a change at commit time |

Write tools are permission-gated server-side; a read-only key can `ask` but cannot capture.

**Make your agent use the write path** — three lines in your `CLAUDE.md` turn debugging sessions into documentation: see [Teach Your Agent](../../docs/agents.md).

## License

MIT — this crate is fully open source so you can audit exactly what runs in your environment and what leaves it.
