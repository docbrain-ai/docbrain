# Coding Agents — Teach Your Agent to File Docs

Your coding agent already has DocBrain's tools. The [`docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp) server (MIT, in `crates/`) gives Claude Code, Cursor, and any MCP-compatible editor four tools:

| Tool | Direction | What it does |
|------|-----------|--------------|
| `docbrain_ask` | read | Cited answers from your org's memory, in the editor |
| `docbrain_suggest_capture` | read | Checks whether documentation gaps exist for a file or function |
| `docbrain_capture` | write | Files a knowledge fragment (a decision, fix, or caveat) tied to a file and line range |
| `docbrain_commit_capture` | write | Captures the *why* behind a change at commit time |

Most teams wire these up and only ever use `ask`. The write path is where the leverage is: **the moment your agent helps you fix something is the one moment the knowledge exists, is fresh, and costs nothing to keep.** Sessions end, terminal scrollback dies, and the fix your agent found never reaches the teammate who hits the same error next month — unless the agent files it.

The missing piece is not a feature. It's standing instructions.

## Setup

Register the MCP server in your editor (Claude Code shown; Cursor and others use their equivalent MCP config):

```bash
claude mcp add docbrain \
  --env DOCBRAIN_API_KEY=db_sk_... \
  --env DOCBRAIN_SERVER_URL=https://docbrain.your-org.internal \
  -- docbrain-mcp
```

!!! note "Use a scoped key"
    Give the agent a key with capture permission but nothing more. Write tools
    are permission-gated server-side — a read-only key can `ask` but cannot
    file captures.

## The snippet

Add this to your project's `CLAUDE.md` (or global `~/.claude/CLAUDE.md`):

```markdown
## DocBrain
When we resolve an error, discover non-obvious behavior, or make a decision a
future engineer would need, do this before the task ends:
1. Call docbrain_suggest_capture for the files involved.
2. If it reports a gap, draft a 3–5 line capture — what broke, the fix, the
   trap to avoid — and show it to me for approval before calling
   docbrain_capture.
Never include secrets, tokens, hostnames from .env files, or customer data in
a capture. When in doubt, leave it out.
```

Cursor users: the same text goes in `.cursor/rules/docbrain.mdc` with `alwaysApply: true`.

## What happens

1. You and your agent fix something real.
2. The agent asks DocBrain whether that knowledge already exists (`docbrain_suggest_capture` — a corpus check, not a guess).
3. If the org doesn't have it, the agent drafts a capture and **asks you first**. You see exactly what leaves the machine.
4. Approved captures land as fragments in the normal review pipeline — space owners, quality gates, nothing auto-publishes.

## Privacy properties

- **Nothing is automatic.** The agent proposes; a human approves every capture, visibly, in the session.
- **The session never uploads.** Only the approved 3–5 line capture crosses the wire — not your transcript, not your code, not your prompts.
- **The client is auditable.** Every line of code that touches your session is MIT-licensed in [`crates/docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp).
- **Captures are attributed and reviewed** like any other fragment — the same governance that applies to Slack captures applies here.

## Honest limitations

Agents follow standing instructions probabilistically — some sessions will forget to check. Treat the snippet as a habit-builder, not a guarantee: teams that also mention it in code review ("did the agent capture this?") see far higher capture rates. If your editor supports session-end hooks, a one-line reminder hook makes the check near-deterministic.
