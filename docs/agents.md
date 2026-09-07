# Coding Agents — Teach Your Agent to File Docs

Your coding agent already has DocBrain's tools. The [`docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp) server (MIT, in `crates/`) gives Claude Code, Cursor, and any MCP-compatible editor eleven tools. These five are the ones this page's workflow uses; the [full table](#all-eleven-tools) is at the end:

| Tool | Direction | What it does |
|------|-----------|--------------|
| `docbrain_context` | read | What the org already knows about the files you are about to change — decisions, caveats and constraints captured against those exact paths, with a warning first if any of it has since gone stale |
| `docbrain_ask` | read | Cited answers from your org's memory, in the editor |
| `docbrain_suggest_capture` | read | Checks whether documentation gaps exist for a file or function |
| `docbrain_annotate` | write | Files a knowledge fragment (a decision, fix, or caveat) tied to a file and line range |
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

## Capturing Knowledge with Premises

When calling `docbrain_annotate`, agents may include an optional `premises` array to link captured knowledge to verifiable facts about your codebase:

```json
{
  "file_path": "src/handlers/payment.rs",
  "annotation": "This API expects a Stripe webhook signature in X-Stripe-Signature header...",
  "premises": [
    {
      "premise_type": "path",
      "expression": "src/config/stripe_keys.env"
    },
    {
      "premise_type": "path",
      "expression": "docs/webhook-integration.md"
    }
  ]
}
```

**Premise Types (v1):**

- `path` — A file path that must exist in a connected source. DocBrain monitors this path against your git sources' file listings (see `PREMISE_MONITOR_ENABLED`). If the path is verified at capture and later disappears, the annotation fires a `premise.broken` event. Paths never found at capture remain `dormant` — they do not alert.

**Other Premise Types:**

Other `premise_type` values are accepted and recorded but not currently validated. They remain in `dormant` state and never trigger alerts. This allows agents to encode additional premise types for future validation.

**Malformed Premises:**

If an array item is missing required fields (`premise_type`, `expression`) or has invalid types, it is skipped silently — the capture does not fail. This allows robust agent capture even if premise formatting is incorrect.

## The snippet

Add this to your project's `CLAUDE.md` (or global `~/.claude/CLAUDE.md`):

```markdown
## DocBrain
Before editing files you have not worked in before, call docbrain_context with
their repo-relative paths and read what comes back first. Pass bare paths —
`src/auth/session.rs`, not `src/auth/session.rs:42`.

When we resolve an error, discover non-obvious behavior, or make a decision a
future engineer would need, do this before the task ends:
1. Call docbrain_suggest_capture for the files involved.
2. If it reports a gap, draft a 3–5 line capture — what broke, the fix, the
   trap to avoid — and show it to me for approval before calling
   docbrain_annotate.
Never include secrets, tokens, hostnames from .env files, or customer data in
a capture. When in doubt, leave it out.
```

Cursor users: the same text goes in `.cursor/rules/docbrain.mdc` with `alwaysApply: true`.

## What happens

1. Before your agent edits a file, it asks what the org already knows about it (`docbrain_context`). A caveat a teammate filed last quarter surfaces *before* the mistake, not in the post-mortem.
2. You and your agent fix something real.
3. The agent asks DocBrain whether that knowledge already exists (`docbrain_suggest_capture` — a corpus check, not a guess).
4. If the org doesn't have it, the agent drafts a capture and **asks you first**. You see exactly what leaves the machine.
5. Approved captures land as fragments in the normal review pipeline — space owners, quality gates, nothing auto-publishes.

## All eleven tools

The five above are the ones this page's workflow uses. The MCP server declares
eleven; the rest are the same capabilities the API and CLI expose, reachable
from the editor. Direction is read unless marked write — write tools require
`editor` role or above, and the server refuses them below that.

| Tool | Direction | What it does |
|------|-----------|--------------|
| `docbrain_context` | read | What the organization already knows about the files you are about to change — decisions, caveats and constraints against those exact paths, with a stale-knowledge warning first |
| `docbrain_ask` | read | Ask a question about your organization's knowledge; cited answer, or an honest "not in the record" |
| `docbrain_incident` | read | Incident-mode search — prioritises runbooks, past incident resolutions, on-call procedures and troubleshooting guides over general documentation |
| `docbrain_suggest_capture` | read | Whether a documentation gap exists for a file or function, before you write a capture nobody needs |
| `docbrain_freshness` | read | Freshness report — which documentation has gone stale, and how stale |
| `docbrain_autopilot_gaps` | read | Documentation gaps Autopilot has detected and clustered from unanswered questions |
| `docbrain_autopilot_summary` | read | Autopilot status: total, open and critical gaps, drafts generated, drafts published |
| `docbrain_feedback` | read | Submit feedback on an answer. Read-direction because it changes no documentation — it feeds answer quality |
| `docbrain_annotate` | **write** | File a knowledge fragment — a decision, fix or caveat — tied to a file and line range |
| `docbrain_commit_capture` | **write** | Capture the *why* behind a change at commit time, grounded in the diff and the commit message |
| `docbrain_autopilot_generate` | **write** | Generate a documentation draft for a specific gap cluster. Routes to human review; nothing publishes unattended |

Nothing here can change your source systems. There is no tool that edits a
Confluence page, closes a ticket or posts to a channel — the write tools write
knowledge fragments into DocBrain's own review queue and nowhere else.

## Privacy properties

- **Nothing is automatic.** The agent proposes; a human approves every capture, visibly, in the session.
- **The session never uploads.** Only the approved 3–5 line capture crosses the wire — not your transcript, not your code, not your prompts.
- **The client is auditable.** Every line of code that touches your session is MIT-licensed in [`crates/docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp).
- **Captures are attributed and reviewed** like any other fragment — the same governance that applies to Slack captures applies here.

## Honest limitations

Agents follow standing instructions probabilistically — some sessions will forget to check. Treat the snippet as a habit-builder, not a guarantee: teams that also mention it in code review ("did the agent capture this?") see far higher capture rates. If your editor supports session-end hooks, a one-line reminder hook makes the check near-deterministic.
