# Slack DM Privacy Policy

How DocBrain handles direct-message content returned by the Slack MCP integration.

## Summary

The Slack MCP server can return DM content (1:1 direct messages and
multi-person group DMs) to tools that have the right OAuth scopes. By
default, **DocBrain does not request those scopes** and **its runtime
strips DM content even if an operator re-adds the scopes on a fork**.
Operators who explicitly want DM content in their DocBrain corpus must
opt in via `DOCBRAIN_DM_PERSIST_POLICY=allow`.

## Why this matters

Slack's per-user OAuth model already prevents user A from reading
user B's DMs — A's user-token only sees what A would see in Slack's
UI. **That boundary is not the same as DocBrain's persistence
boundary.** DocBrain captures every MCP tool result into:

- `episodes.cached_sources_json` — the per-`/ask` source blob
- `episodes.answer_text` — the synthesized text, which can quote DM
  content verbatim
- Memory consolidation pipelines — long-lived semantic entities
  built from episodes
- Expert routing / autopilot gap clustering — episodes grouped by
  similarity and surfaced to other users

So when user A's DocBrain query surfaces A's own DMs, that DM content
becomes part of the persistent DocBrain corpus and can re-surface
elsewhere — answer caches, similar-question retrieval for other users
(via entity overlap), the knowledge stream. **The risk isn't that B
can read A's DMs through MCP; it's that A's own DMs leak from Slack
into DocBrain's broader surfaces.**

## What the OSS default does

Two defaults work together:

### 1. Default Slack manifest omits DM scopes

`config/mcp-manifests/slack.yaml` (in the docbrain repo) requests
these OAuth scopes:

- `search:read.public` — public channels
- `search:read.private` — private channels the user is a member of
- `search:read.files` — files

It **does NOT request**:

- `search:read.im` — 1:1 DMs
- `search:read.mpim` — multi-person group DMs

Without these scopes, Slack's API refuses to return DM content to any
tool — regardless of which tool the picker dispatched or how the query
was constructed. Slack enforces this at the scope boundary.

### 2. Runtime redactor (defense in depth)

Even if an operator forks the manifest to add `search:read.im` and
`search:read.mpim`, DocBrain's runtime detects DM entries in tool
results and strips them before they reach any downstream consumer
(synthesis prompt, episodes, memory, audit log).

Detection signals (any one trips the gate):

- `is_im: true` on the message's `channel` object
- `is_mpim: true` on the `channel` object
- `channel.id` starts with `D` (Slack's stable 1:1 DM prefix)
- Entry-level `channel_id` starts with `D` (for flattened payloads)
- Entry-level `is_im` / `is_mpim` flag

What is **not** redacted (preserves the legitimate private-channel
feature):

- Channel IDs with `C` prefix (public channels)
- Channel IDs with `G` prefix that do NOT carry `is_mpim: true`
  (legacy private channels in older Slack workspaces — these
  intentionally pass through)

When the redactor strips an entry, it replaces the entire object
with a sentinel:

```json
{
  "redacted": true,
  "text": "__DOCBRAIN_DM_REDACTED__ DM source — not persisted. Open Slack to view."
}
```

No message body, channel ID, sender, permalink, or timestamp survives.
The sentinel is recognisable to the synthesizer LLM (it can construct
an honest "I found N DMs but they're not surfaced here" response) and
greppable for operators auditing the corpus.

## Where the redactor fires

One seam in the data flow:

```
Gateway dispatch
   → ToolResult bytes
   → [REDACTOR HERE]   ← fanout::apply_redactor()
   → format::render_block()  (the synthesis prompt input)
   → ToolDispatchSummary     (the audit-log size column)
   → episodes.cached_sources_json
   → memory consolidation
   → expert routing
```

Single point of mutation, single audit column reflects the
post-redaction size. Every downstream surface naturally inherits the
redaction — no per-persistence-site filters, no drift risk between
them.

A debug-build canary in `episodic_memory::store_episode` panics if
DM content somehow reaches the persistence boundary (i.e., the
redactor was bypassed or has a bug). The panic is `#[cfg(debug_assertions)]`
only — zero runtime cost in release builds.

## Configuration

Set `DOCBRAIN_DM_PERSIST_POLICY` (or the equivalent helm value
`mcpTools.dmPersistPolicy`) to one of:

### `strict` (default — recommended)

DM-bearing entries are stripped before any downstream sees them. The
LLM still receives a count + redaction marker, so it can construct a
useful response (e.g., "I found 1 message that matched but it's in a
DM — open Slack to view"). The user gets a hint that the answer
might exist; they go to Slack to read the actual content.

Use this in every production deployment unless you have a specific,
audited reason to do otherwise.

### `warn`

The redactor logs a `tracing::warn!` per dispatch when it would have
redacted DM content, but lets the content through unchanged. The
warn line includes the manifest, tool name, and the number of
messages/channels that would have been stripped.

Use this for **staging or debugging only** — e.g., when investigating
why a specific Slack query returns nothing, you want to see whether
the matches existed in DMs but got stripped. Never run production
with `warn`.

### `allow`

The redactor is disabled entirely. No detection, no logging. DM
content flows through unredacted into every downstream surface.

This is the explicit foot-gun for operators who:

1. Have a documented reason to materialise DM content into the
   DocBrain corpus (e.g., legal e-discovery on their own workspace,
   security investigation).
2. Have forked the OSS Slack manifest to re-add the
   `search:read.im` and `search:read.mpim` scopes.
3. Understand that DM content will become part of memory
   consolidation, expert routing, and the knowledge stream — and
   may surface to other DocBrain users via entity overlap.

If you flip this to `allow`, document the reason in your deployment's
change log and consider whether you also need to restrict who can
query the Slack MCP tools (the `rbac.required_role` field on the
manifest).

## Helm

In your values file:

```yaml
mcpTools:
  enabled: true
  # Strict by default; uncomment to override.
  # dmPersistPolicy: "allow"
```

For operators forking the manifest to re-add DM scopes, see
`config/mcp-manifests/slack.yaml` in the docbrain repo. The OSS
chart's `files/mcp-manifests/slack.yaml` is a symlink to the same
file; customer forks should replace the file in their values overlay
or build a custom image.

## What this policy does NOT do

- **Does not prevent user A from seeing their own DMs in DocBrain's
  /ask UI** when `allow` is set. A intentionally granted Slack the
  OAuth scope; A can read their own DMs through DocBrain's UI just
  as they can in Slack's UI. The policy controls *persistence*, not
  ephemeral display.
- **Does not block tool dispatch.** The Slack tool still runs, still
  returns whatever Slack gives back. The redactor edits the bytes
  after dispatch but before they reach any DocBrain-side consumer.
- **Does not redact other manifests.** Only the `slack` manifest_id
  is registered with the redactor today. If you add another MCP
  integration that returns privacy-classified content, you'll need
  to register a redactor for it.
- **Does not retroactively scrub historical episodes.** Flipping from
  `allow` → `strict` only affects new tool dispatches. Existing
  episodes that contain DM content from a prior `allow` run stay
  in `episodes.cached_sources_json` and `episodes.answer_text`
  until you scrub them manually.

## Verifying the policy is active

A working `strict` deployment, on a query like
`Search Slack for "deploy" — include DMs`:

1. The tool dispatcher selects `slack.slack_search_public_and_private`
   (the tool that *can* search DMs).
2. The picker's "Why these tools" output cites the tool.
3. The redactor logs a `tracing::info!` if any DM entries were
   stripped:
   ```
   manifest=slack tool=slack.slack_search_public_and_private
   messages_redacted=N channels_redacted=0
   DM redactor stripped sensitive entries before downstream consumers
   ```
4. The answer text references "redacted" or directs the user to
   Slack, rather than quoting DM content verbatim.
5. `episodes.cached_sources_json` for that episode contains the
   sentinel string `__DOCBRAIN_DM_REDACTED__`, not raw DM content.

If you see DM content in step 4 or 5, the policy is not in effect —
check the deployed `DOCBRAIN_DM_PERSIST_POLICY` env var.

## Migration path for existing deployments

If you ran an earlier docbrain release with the Slack manifest that
DID request DM scopes (`search:read.im` / `search:read.mpim`):

1. **Update the manifest** to drop the DM scopes (already done in
   the OSS default — sync your fork if applicable).
2. **Revoke active Slack OAuth tokens** so existing connections force
   re-OAuth into the reduced scope set:
   ```sql
   UPDATE mcp_oauth_tokens
   SET revoked_at = now()
   WHERE manifest_id = 'slack' AND revoked_at IS NULL;
   ```
3. **(Optional) Scrub historical DM content** from episodes that
   contain it. There's no automatic scrubber today — operators with
   real DM contamination should run a one-off query targeting
   episode IDs known to have called `slack_search_public_and_private`
   while DM scopes were active.

Step 1 alone closes the future-DM-leak path. Step 2 closes the
"existing token still has DM scope" gap. Step 3 is only needed if
your prior corpus has DM content you want gone.
