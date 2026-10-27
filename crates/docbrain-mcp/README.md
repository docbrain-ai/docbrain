# docbrain-mcp

MCP server that connects Claude Code, Cursor, and any MCP-compatible editor to your organization's [DocBrain](https://github.com/docbrain-ai/docbrain) instance.

**Most editors do not need this package.** A DocBrain server (1.6.4 or later) serves these same tools over HTTP at your DocBrain address followed by `/api/v1/mcp`, and Claude Code connects there by signing in through the browser, with no key: see [Coding Agents](https://github.com/docbrain-ai/docbrain/blob/main/docs/agents.md#setup). This package is the stdio connector, for an editor that only starts local programs.

**This is the connector, not the product.** It's a thin stdio adapter that translates MCP tool calls into REST calls against a DocBrain server — all data and logic live server-side. Without a running DocBrain server it does nothing (and will tell you so at startup).

```
Your editor (agent) ──HTTP──────────────────────> your DocBrain server  /api/v1/mcp
Your editor (agent) ──stdio──> docbrain-mcp ──HTTPS──> your DocBrain server
```

## Prerequisites

A self-hosted DocBrain server — deploy one in ~5 minutes: [Quickstart](https://github.com/docbrain-ai/docbrain#quickstart)

## Setup

Over HTTP, with no package (Claude Code shown; then run `/mcp`, choose `docbrain` and sign in):

```bash
claude mcp add --transport http docbrain https://docbrain.your-org.internal/api/v1/mcp
```

Over stdio, with this package:

```bash
claude mcp add docbrain \
  --env DOCBRAIN_API_KEY=db_sk_... \
  --env DOCBRAIN_SERVER_URL=https://docbrain.your-org.internal \
  -- docbrain-mcp
```

Create a scoped key with `docbrain token create --name "MCP Key" --role viewer` (use a key with capture permission if you want the write tools below).

## Tools

Twelve, all declared by the server. Write tools are permission-gated
server-side: a read-only key can `ask` but cannot capture.

### Read

| Tool | What it does |
|------|--------------|
| `docbrain_context` | What your organisation already decided about specific files. Takes repo-relative paths, returns the decision, the constraint, and — first, and dated — any warning that a premise it rests on has stopped being true. The tool to call *before* an edit. |
| `docbrain_ask` | Cited answers from your org's memory, in the editor, using the live tools connected to DocBrain. Each answer ends with a `session_id` line; pass that id back as `session_id` to ask a follow-up in the same conversation, omit it to start a new one |
| `docbrain_incident` | Incident-mode search: prioritises runbooks, past incident resolutions, on-call procedures and troubleshooting guides over general documentation |
| `docbrain_freshness` | How current the knowledge about a file or area is, and what has gone stale |
| `docbrain_suggest_capture` | Checks for documentation gaps around a file or function — a corpus check, not a guess |
| `docbrain_autopilot_gaps` | Where the corpus is missing documentation it should have |
| `docbrain_autopilot_summary` | What Autopilot has drafted and what is awaiting review |

### Write

| Tool | What it does |
|------|--------------|
| `docbrain_annotate` | Files a fix, decision or caveat as a fragment. Optional `gap_id` names the open question (a gap) it answers and closes it |
| `docbrain_commit_capture` | Captures the *why* behind a change at commit time. Optional `gap_id`, as above |
| `docbrain_revise` | Edits (`text` and/or `premises`) or withdraws (`withdraw_reason`) a capture you recorded, by `id`. Only the author can edit; a capture already used in an answer can only be withdrawn. An older binary lacks this tool and fails closed |
| `docbrain_autopilot_generate` | Drafts documentation for a gap Autopilot found |
| `docbrain_feedback` | Records whether an answer was useful, so retrieval learns |

**Make your agent use the write path** — a few lines in your `CLAUDE.md` turn debugging sessions into documentation: see [Teach Your Agent](../../docs/agents.md).

## Configuration

| Variable | Meaning |
|---|---|
| `DOCBRAIN_SERVER_URL` | Your instance. Defaults to `http://localhost:3000`, the server's own port; with Docker Compose use `http://localhost:3001`. **Not** `DOCBRAIN_API_URL`, which is the HTTP/CI variable — setting the wrong one leaves the server on its default port and the error names a port you never configured. |
| `DOCBRAIN_API_KEY` | From `docbrain token create --name "MCP" --role viewer`. Required. |

The connector reads these from the environment your editor starts it with, normally the
config's `env` block, and never from a `.env` file on its own. To keep them in a file of your
own, add `"args": ["--docbrain-env", "/full/path/mcp.env"]` to your user-level MCP config: only
that file is read, a relative path is refused, a non-empty value in the `env` block wins over the
file, and the file may set only DocBrain's own variables the connector reads, the two above (any
other variable, or any `$`, refuses it). Do not point an editor's own `envFile` setting at a file
inside the project: that sets every variable in it for the connector.

Install it once (`npm install -g docbrain-mcp`, or the release binary) and start
`docbrain-mcp`. Starting it with `npx` from a project makes npm read that project's `.npmrc`.

Ready-made configs for Claude Code, Cursor and VS Code are in
[`examples/mcp-configs/`](../../examples/mcp-configs/), including which file
each editor reads and why the top-level key differs between them.

## License

MIT — this crate is fully open source so you can audit exactly what runs in your environment and what leaves it.
