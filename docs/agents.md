# Coding Agents — Knowledge That Compounds Across People

Someone works out why a thing is the way it is. Six weeks later a person on
another team, who has never met them, opens that file — and has no way to know it
was ever worked out. Neither does their agent, which proposes the approach that
was already ruled out.

That is not a session-memory problem and a rules file does not fix it. A
`CLAUDE.md` holds what you remembered to write, in your repo, for your agent: it
does not cross people, it does not expire when the system changes, and it carries
no source anyone can check. Your coding agent is, however, the best instrument
for both halves of the actual fix — it is present at the second the knowledge is
created, and it is the thing asking for it at the moment of the next change.

Your coding agent already has DocBrain's tools. The [`docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp) server (MIT, in `crates/`) gives Claude Code, Cursor, and any MCP-compatible editor eleven tools. These five are the ones this page's workflow uses; the [full table](#all-eleven-tools) is at the end:

| Tool | Direction | What it does |
|------|-----------|--------------|
| `docbrain_context` | read | What the org already knows about the files you are about to change — decisions, caveats and constraints captured against those exact paths, with a warning first if any of it has since gone stale |
| `docbrain_ask` | read | Cited answers from your org's memory, in the editor |
| `docbrain_suggest_capture` | read | Checks whether documentation gaps exist for a file or function |
| `docbrain_annotate` | write | Files a knowledge fragment (a decision, fix, or caveat) tied to a file and line range |
| `docbrain_commit_capture` | write | Captures the *why* behind a change at commit time |

Most teams wire these up and only ever use `ask`. The write path is where the leverage is: **the moment your agent helps you fix something is the one moment the knowledge exists, is fresh, and costs nothing to keep.** Sessions end, terminal scrollback dies, and the fix your agent found never reaches the teammate who hits the same error next month — unless the agent files it.

The missing piece is not a feature. It's standing instructions — and a skill that carries them, so nobody has to remember to write them.

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

## The skill

`docbrain-capture` is a Claude Code skill that does the write path for you. When a conversation has
produced knowledge that is written nowhere — a decision and why, a fix that took digging, a caveat, a
procedure (how we deploy, rotate, roll back, get access), how a system is actually wired — it:

1. checks DocBrain first (`docbrain_context` for the files concerned, or `docbrain_ask`), and offers an
   update instead of a duplicate when the knowledge already exists;
2. drafts one capture per fact in your team's words: what, why, how — the exact command, path, flag or
   value — anchored to the file and line range it concerns, with the premises it rests on declared
   (paths that must exist), so DocBrain can flag it the day one of them disappears;
3. shows the draft exactly as it will be sent and waits for your yes;
4. writes it with `docbrain_annotate` (or `docbrain_commit_capture` for the *why* of a commit) and
   reports where it landed: indexed, or queued for review.

It offers itself once, at a natural breakpoint, never mid-task, and a "no" ends it. Nothing leaves the
session unapproved. It never captures secrets, tokens, hostnames from `.env` files, customer or
personal names, logs or chatter.

### Install it

For the whole team, from the repository you work in:

```bash
claude plugin marketplace add docbrain-ai/docbrain --scope project
claude plugin install docbrain@docbrain --scope project
```

Both commands write to `.claude/settings.json` — the marketplace under `extraKnownMarketplaces`, the
plugin under `enabledPlugins`; commit that file and everyone who trusts the project has the skill. Invoke it as `/docbrain:docbrain-capture`, or say
"capture this", "write this down", "document how we did that".

Prefer a plain directory? Copy
[`plugins/docbrain/skills/docbrain-capture`](https://github.com/docbrain-ai/docbrain/tree/main/plugins/docbrain/skills/docbrain-capture)
into your repository's `.claude/skills/` and invoke it as `/docbrain-capture`. The skill's template and
three finished captures are in its `resources/` directory.

What counts as adoption is not the number of captures but the number that later answer a question or
get corrected by their premises — measure that, not volume.

## The snippet

Without the skill (Cursor, other editors, or a team that prefers prose), add this to your project's `CLAUDE.md` (or global `~/.claude/CLAUDE.md`):

```markdown
## DocBrain
Before editing files you have not worked in before, call docbrain_context with
their repo-relative paths and read what comes back first. Pass bare paths —
`src/auth/session.rs`, not `src/auth/session.rs:42`.

When we resolve an error, discover non-obvious behavior, or make a decision a
future engineer would need, do this before the task ends:
1. Call docbrain_context for the files involved and read what is already
   recorded there.
2. If it is not there, draft a short capture — what broke, the fix, the trap
   to avoid — and show it to me for approval before calling docbrain_annotate.
Never include secrets, tokens, hostnames from .env files, or customer data in
a capture. When in doubt, leave it out.

If the docbrain tools are not available in this session, say so plainly and
stop there. Do not fall back to searching the repository and then report that
nothing is recorded — captured decisions do not live in the repo, so an
absent connector looks exactly like an empty knowledge base and is not one.
```

Cursor users: the same text goes in `.cursor/rules/docbrain.mdc` with `alwaysApply: true`.

### Open the workspace at the folder that holds the config

MCP connector config is **workspace-root scoped** in every editor that reads it
— `.mcp.json`, `.vscode/mcp.json`, `.cursor/mcp.json` are all found relative to
the folder you opened, not to the file you are editing. Open a monorepo's parent
directory instead of the service folder and the server never loads.

That failure is silent and it is worse than an error, which is why the last
paragraph of the snippet exists. Observed: with the connector unloaded, an agent
searched the repository, found no captured decisions there — correctly, they are
not stored there — and answered *"the org knows nothing about this file yet"*
about a file carrying a live pinned decision and an out-of-date warning. A
missing connector reads as an empty knowledge base unless the agent is told to
distinguish them.

Check before you rely on an answer: your editor's MCP panel should list
`docbrain` and its tools. If it does not, you are looking at the wrong workspace
root, not at an org that has recorded nothing.

## What happens

1. Before your agent edits a file, it asks what the org already knows about it (`docbrain_context`). A caveat a teammate filed last quarter surfaces *before* the mistake, not in the post-mortem.
2. You and your agent fix something real.
3. The agent asks DocBrain whether that knowledge already exists (`docbrain_context` for the files involved — a lookup in the record, not a guess).
4. If the org doesn't have it, the agent drafts a capture and **asks you first**. You see exactly what leaves the machine.
5. Approved captures land as fragments, anchored to the file and line range they came from. A capture that declares a premise becomes checkable: DocBrain re-verifies it against your codebase and flags it the moment it stops being true, without anyone reviewing anything. A capture anchored to nothing checkable is queued for a human instead of served.

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
knowledge fragments into DocBrain's own corpus and nowhere else.

## Privacy properties

- **Nothing is automatic.** The agent proposes; a human approves every capture, visibly, in the session.
- **The session never uploads.** Only the approved capture crosses the wire — not your transcript, not your code, not your prompts.
- **The client is auditable.** Every line of code that touches your session is MIT-licensed in [`crates/docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp).
- **Captures are attributed and falsifiable.** Every fragment records who filed it, and a capture is only served unreviewed when it is anchored to real code — the trust level is derived by the server from those anchors, never from anything the client claims about itself.

## Honest limitations

Agents follow standing instructions probabilistically — some sessions will forget to check. Treat the snippet as a habit-builder, not a guarantee: teams that also mention it in code review ("did the agent capture this?") see far higher capture rates. If your editor supports session-end hooks, a one-line reminder hook makes the check near-deterministic.
