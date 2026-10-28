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

Your coding agent already has DocBrain's tools. DocBrain serves twelve of them over MCP to Claude Code, Cursor and any MCP-compatible editor, from one address on your server; the tool code is MIT-licensed in [`crates/docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp). These five are the ones this page's workflow uses; the [full table](#all-twelve-tools) is at the end:

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

DocBrain serves MCP at one address: your DocBrain address followed by `/api/v1/mcp`, exactly, with no
trailing slash. With Docker Compose that is `http://localhost:3001/api/v1/mcp`. While Sign-in for apps
is on, your account menu › **Connected apps** in DocBrain shows it with a Copy button.

**Claude Code** (2.1.186 or later). Install the [plugin](#install-it): it connects the server and adds the
capture skill. Or connect the server alone:

```bash
claude mcp add --transport http --scope project docbrain https://docbrain.your-org.internal/api/v1/mcp
```

Then sign in once: run `/mcp`, choose `docbrain` (`plugin:docbrain:docbrain` when it came with the
plugin) and sign in. Your browser opens DocBrain; sign in
(approving an app needs a sign-in from the last 10 minutes) and click **Allow**. There is no API key
and nothing secret in any file, so the `.mcp.json` that `--scope project` writes can be committed for
the whole team. Over SSH or without a browser, run `claude mcp login docbrain --no-browser`
(`claude mcp login plugin:docbrain:docbrain --no-browser` if you installed the plugin).

The agent acts as you, with your role and never more: a `viewer` can read and ask, captures need
`editor`. The connection ends after 30 days unused or 90 days after you allowed it (an admin can
change the 90 with `MAX_PAT_DAYS`), and you can revoke it any time under **Connected apps**.

**Cursor, VS Code and other editors.** Point them at the same address with an API key read from your
environment, so no key is written in the file. Create the key in DocBrain under your account menu ›
**My API keys** (Editor, so captures work), or with `docbrain token create --name "MCP" --role editor`.
Ready-made files are in [`examples/mcp-configs/`](https://github.com/docbrain-ai/docbrain/tree/main/examples/mcp-configs):

```jsonc
// .cursor/mcp.json
{
  "mcpServers": {
    "docbrain": {
      "url": "http://localhost:3001/api/v1/mcp",
      "headers": { "Authorization": "Bearer ${env:DOCBRAIN_API_KEY}" }
    }
  }
}
```

An editor that signs in with an MCP *client ID metadata document* can sign in through the browser
instead, as Claude Code does. If DocBrain's page then says the app is not allowed yet, that app returns
to a website rather than to your computer, and an admin allows it once under **Settings › Sign-in for
apps**.

**Scripts and CI.** A machine with no browser connects with a key. Keep the key out of the committed
`.mcp.json`: Claude Code does not fall back to browser sign-in for a server that has an `Authorization`
header, so everyone using that file would need the key. On the CI machine, add the server at local
scope instead:

```bash
claude mcp add --transport http docbrain https://docbrain.your-org.internal/api/v1/mcp --header "Authorization: Bearer $DOCBRAIN_API_KEY"
```

`claude -p` on a machine where you signed in once uses that sign-in.

**Editors that only start local programs.** The [`docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp)
connector speaks MCP over stdio for them: `npm install -g docbrain-mcp`, with `DOCBRAIN_SERVER_URL` and
`DOCBRAIN_API_KEY` in its environment.

!!! note "Use a scoped key"
    When you connect with a key, give the agent one with capture permission but nothing more. Write
    tools are permission-gated server-side: a read-only key can `ask` but cannot file captures.

### Sign-in for apps

Signing in from the editor is DocBrain acting as an OAuth 2.1 authorization server for its own MCP
endpoint, as the MCP specification describes. It is **on by default with Docker Compose** and off on
Kubernetes until an admin turns it on:

- Docker Compose: on; `DOCBRAIN_OAUTH_SERVER=off` in `.env` turns it off.
- Kubernetes: `oauthServer.enabled: true` in the Helm values.

It needs `DOCBRAIN_WEB_BASE_URL` (Helm: `webBaseUrl`) to be the one address of the console and the API:
https, or `http://localhost` for development. Every SSO provider must land people on that same address.
If something is missing it stays off, and the server log and **Settings › Sign-in for apps** say why.

**Settings › Sign-in for apps** shows whether it is on, lets an admin pause new sign-ins (**Allow new
sign-ins**), checks from your browser that apps can find sign-in (**Check now**), and lists sign-in
problems from the last 7 days. It also holds:

- **Web apps you allow**: an app that returns to your own computer, like Claude Code, may ask by default;
  an app that returns to a website needs an entry here, by its client ID.
- **App documents registered here**: DocBrain reads each app's metadata document from the internet
  (public addresses only, 5 seconds, 5 KB). Without outbound access, register the document here, or set
  `DOCBRAIN_OAUTH_DOCUMENT_PROXY` to fetch through your proxy.
- **Connected apps in this DocBrain**: each person and app, with **Revoke** and **Revoke all**. After a
  restore from backup, use **Revoke all**.

A connection ends when its person or an admin revokes it, when the person is disabled, after
30 days unused, or 90 days after it was allowed. Signing out of the console does not end it.

## Capturing Knowledge with Premises

When calling `docbrain_annotate` or `docbrain_commit_capture`, agents may include an optional `premises` array to link captured knowledge to verifiable facts about your codebase:

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

**Commit captures declare theirs by listing the files:**

`docbrain_commit_capture` records why a change was made. Every path in its `file_paths` list becomes a `path` premise on the server, so the capture is checked against the files it describes and is flagged when one of them moves or disappears — send the file list, or nothing will ever be able to tell whether the reasoning still holds. Its `premises` array is for what the reasoning rests on beyond the files the commit touched.

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
   reports what DocBrain answered: the capture's id and any warning — it is served at once.

It offers itself once, at a natural breakpoint, never mid-task, and a "no" ends it. Nothing leaves the
session unapproved. It never captures secrets, tokens, hostnames from `.env` files, customer or
personal names, logs or chatter.

### Install it

For the whole team, from the repository you work in:

```bash
claude plugin marketplace add docbrain-ai/docbrain --scope project
claude plugin install docbrain@docbrain --scope project
```

The install asks for your DocBrain MCP address (see [Setup](#setup)); then run `/mcp`, choose
`docbrain` and sign in. Pass `--config mcp_url=…` to answer without a prompt. Both commands write to
`.claude/settings.json` — the marketplace under `extraKnownMarketplaces`, the plugin under
`enabledPlugins`; commit that file and everyone who trusts the project has the skill, and each person
signs in as themselves. Invoke it as `/docbrain:docbrain-capture`, or say
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
5. A capture you approve is served at once, anchored to the file and line range it came from, and labelled with who wrote it and what checks it. One whose declared premises all hold reads *Checked against code*: DocBrain re-verifies it against your codebase, flags it the moment it stops being true, and tells you once. One with a claim that does not hold names why: *Not found in any connected repository* (a path no repository has, say) or *Does not match the connected repository* (a version that differs), and the save-time reply says which. One where no claim fails but some cannot be checked now (their repository is disconnected, say) reads *Cannot be checked now*, even when its other claims hold. One anchored to nothing checkable is served too, labelled *Not checked against code*, so every reader knows.

A capture is refused when a premise it declares names a file a connected repository was recorded
holding and none holds now: the reply is `422 premise_removed` naming the path, and the agent corrects
or drops that premise and shows you the draft again. A file you are adding has no such record and is
never refused.

## What a capture never holds

A capture is shared with everyone who can read its space, so DocBrain checks every capture for secrets before it is
saved. Two checks run. A fixed check recognises the shapes of private keys, access tokens and API keys in published
formats, connection strings and links with a password, signed links and long random-looking values, also through
common encodings and tricks such as invisible characters, look-alike letters and spaced-out text, and together with
your other captures from the last 24 hours. Then the language model your DocBrain is configured with reads the
capture's text and lists any password, key or token written in it, including one written in plain words and values
the text calls test, example or sample values. Write where a secret lives (`${DB_PASSWORD}`, `<password>`, "in the
team vault") instead of a value. The capture's text is sent to that model's provider for this check.

If a capture you write holds a secret, it is not saved, and you are told what kind it looked like. After three of
these in ten minutes, capturing pauses for ten minutes. When DocBrain writes a summary for you (from a chat thread or a
CI run), it removes the secret, keeps the rest, and tells you what kind was removed.

If the model cannot be reached, a capture is saved after the fixed check and checked by the model as soon as it is
back; if a secret is then found, the capture is withdrawn (a summary has the secret removed), the answers that used it
are cleaned, and you are told. Until then it may be served. When the detection rules, the model or its instructions
change, every existing capture is checked again. Every answer and reply is checked again for the fixed shapes just
before it is sent; plain-word passwords are checked in captures, not in answers.

**What we cannot promise:** a secret described in words, a value deliberately disguised in small pieces, or a value
deliberately split around a note can pass. Every capture says who wrote it, is visible only to its space, and can be
withdrawn at once or erased by an admin. Never put a secret in a capture. If one was ever served, treat it as exposed
and rotate it.

### Refused or redacted

- **A capture a person writes** (the console, the CLI, `docbrain_annotate`, `docbrain_commit_capture`,
  `docbrain_revise`) is **refused** when it holds a secret: nothing is saved, and the reply names the kind of secret
  and the field it was in, the line, and which family of rules matched (for DocBrain's own rules, also the masked
  shape of the match, such as `aaaa/9999…`), never the value. An agent reports the refusal to the person exactly as
  given and asks whether that line holds a secret; it never edits or resends the text on its own. Repeated refusals
  count toward the pause, and the same text sent again is refused again.
- **A summary DocBrain writes** (a chat-thread capture, a CI run analysis) is **redacted**: the secret is replaced by
  a marker such as `[removed: password]`, the rest is kept, and the person who asked is told the kind that was
  removed. These never pause anyone.

### What you are told

A notice reaches the console's notifications page only when there is something for you to do, and it says what and
where: every notice has one action, which opens the place to do it.

- **Your capture lost a value** (it was withdrawn because it held what looks like a secret, or the value was removed
  from a summary DocBrain wrote for you): rotate the value if it was real. The notice opens the capture; a chat
  thread captured as a document has no capture to open, so its notice only says what to do.
- **While DocBrain or an admin is still checking a capture you wrote, you are not notified.** My captures shows it
  withdrawn, with "waiting for an admin" (pinned at the top) or "until it can be checked again". You are told as soon
  as a value is removed, even if something else in the capture still waits, because that is when there is something
  to rotate; if the admin restores it, it is simply served again.
- **Admins** have one standing notice while captures wait for their decision: "N captures wait for your decision",
  opening the waiting list. It counts every capture waiting for a decision, the same list the console shows any admin
  who can sign in. It is updated in place, comes back unread only when another capture starts
  waiting, and goes away when nothing waits; it does not expire after 30 days like other notices. Marking it read
  keeps it quiet; dismissing it while captures still wait brings it back, since the decision is still open.
- Admins are also asked to rotate a value removed from a capture that had already been served (one notice per
  capture, opening it, whichever check or decision removed it, including a purge while it waited; a value found
  only in a withdraw reason, which readers never see, asks only its author), to rotate a value found in a signed evidence record, and to fix the model's provider while
  the secret check's model is unavailable (the notice goes once it has caught up).

### The model check

The second check uses your configured fast model. Each capture's text is sent to that model's provider; choose a
local provider if capture text must not leave your network. The check's time budget is
`CAPTURE_SECRET_CHECK_TIMEOUT_SECS` (default 10 seconds; see the [configuration reference](configuration.md#secret-check-on-captures)).
A weaker model finds less: the fixed check does not depend on the model, the plain-word check does.

### Custom secret rules

An admin can add a rule for a secret shape your organisation uses (Govern → Quality, rule type **Secret Pattern**).

- **Dry run before save.** A rule is checked before it can be saved: DocBrain shows how often it matches ordinary
  notes and existing captures, and refuses a rule that is invalid, too slow, or would refuse too many ordinary notes.
  The save goes through only after the admin has seen that check. The same applies to enabling or importing a rule.
- **Second decision on what it withdrew.** When a saved rule matches existing captures, they are withdrawn and wait
  for an admin's decision on that rule version ("Waiting for your decision"): **Remove the values** (Confirm) removes
  them for good — a capture a person wrote stays withdrawn without them, a summary DocBrain made is served again — or
  **Restore these captures** (Decline) serves them again as written. An admin can also confirm or restore a single
  capture from the captures list.
- **Restored values are shown for that rule version.** After a restore, the values that rule matched in the
  restored captures when the decision was made are no longer masked by it, in those captures and in the answers and
  sources served from them, for as long as the rule stays at that version (see the limits below).
- **A new rule version checks again.** Editing a rule makes a new version: every capture is checked again under it,
  and restorations made under the old version do not carry over.
- **A person must decide.** An admin key with no user behind it (a keyless key) cannot create, change, enable,
  delete or import a secret rule, and cannot confirm or restore what one withdrew; use a key created by a signed-in
  admin. Every decision names the admin who made it.

### What DocBrain keeps of a finding

Never the secret. A finding is stored as a keyed fingerprint (HMAC-SHA256 under the deployment's fingerprint key), so
someone holding the database or a backup cannot guess a short value from it. The key is generated on first start
(Docker Compose: the `docbrain-keys` volume; Helm: its own Secret, kept on uninstall) unless you supply one, and is never stored in
the database. Back it up, separately from the database backups: a lost or changed key reopens every restored or
declined finding. See [the fingerprint key](configuration.md#the-fingerprint-key).

### Honest limits

- Detection recognises shapes and, through the model, written credentials. A secret described in words, a short
  value with nothing around it, or one deliberately disguised or split into pieces can pass.
- The model is not deterministic, and a weaker configured model detects less.
- While the model is unreachable or over budget, a capture is saved after the fixed check and may be served until the
  model checks it. Answers already given are cleaned when a secret is found, but replies already sent to chat, MCP
  clients and webhooks cannot be recalled.
- Answers are checked for the fixed shapes only; plain-word passwords are checked in captures, not in answers.
- Ingested documents keep their secrets at their source; DocBrain removes recognised secrets from what it serves,
  and the source system is where they must be removed.
- Links are not followed: a link to a paste or file share that holds a secret is served as a link.
- A restore covers less than everything: it exempts only the values present when the admin decided (a value found
  later at the same rule version stays masked); the question as stored and shown keeps `[removed: …]`; copies
  DocBrain wrote itself (summaries) stay redacted; and passages DocBrain joins from references are checked without
  the exemption. Each of these masks more, never less.
- What contains these limits: a capture is visible only to its space, says who wrote it, and can be withdrawn at once
  or erased by an admin. If a secret was ever served, treat it as exposed and rotate it.

## All twelve tools

The five above are the ones this page's workflow uses. The MCP server declares
twelve; the rest are the same capabilities the API and CLI expose, reachable
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
| `docbrain_revise` | **write** | Edit or withdraw a capture you recorded (its id comes from `docbrain_context`); a summary DocBrain wrote can only be withdrawn |
| `docbrain_autopilot_generate` | **write** | Generate a documentation draft for a specific gap cluster. Routes to human review; nothing publishes unattended |

Nothing here can change your source systems. There is no tool that edits a
Confluence page, closes a ticket or posts to a channel — the write tools write
knowledge fragments into DocBrain's own corpus and nowhere else.

## Privacy properties

- **Nothing is automatic.** The agent proposes; the person who did the work approves every capture, visibly, in the session. No admin queue stands between that yes and the answer.
- **The session never uploads.** Only what a tool sends crosses the wire: a question to answer, the paths to look up, a capture you approved. Never your transcript or your files.
- **Nothing goes to DocBrain's makers.** The plugin and the connector talk only to the DocBrain server your organization runs, at the address you gave them; neither has telemetry.
- **The tool code is auditable.** The tools your editor calls, and their MCP HTTP binding, are the MIT-licensed [`crates/docbrain-mcp`](https://github.com/docbrain-ai/docbrain/tree/main/crates/docbrain-mcp); the DocBrain server runs that same code at `/api/v1/mcp`, and the stdio connector is built from it.
- **Captures are attributed and falsifiable.** Every capture says who wrote its words — *Recorded by <name>*, or *Summarised by DocBrain from <source>* for a summary DocBrain wrote — and what checks it, derived by the server, never from anything the client claims. Its author can edit or withdraw it at any time (delete only until it has been used), an admin can erase it, and a key with no user cannot write one.

## Honest limitations

Agents follow standing instructions probabilistically — some sessions will forget to check. Treat the snippet as a habit-builder, not a guarantee: teams that also mention it in code review ("did the agent capture this?") see far higher capture rates. If your editor supports session-end hooks, a one-line reminder hook makes the check near-deterministic.
