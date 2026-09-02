# MCP Tool Platform

DocBrain answers questions with live data — current Jira ticket status, open GitHub PRs, anything served by an MCP server — cited inline in the same answer as your indexed knowledge. Live tools turn DocBrain from a retrospective search engine into a system that knows the state of the world *right now*.

---

## Overview

Most DocBrain answers come from the ingest pipeline: docs, Slack threads, code, and connector content that was chunked, embedded, and indexed at some point in the past. That works for "how is our auth system designed?" — it does not work for "what's the status of PROJ-123?"

The **MCP Tool Platform** is the answer-time complement. At every `/ask`, after retrieval but before synthesis, an orchestrator can dispatch one or more live tools — calls to external systems that return fresh data. Those results are folded into the synthesis prompt as `<live_data>` blocks, the LLM cites them in the answer, and the UI surfaces a chip on the answer card so the user knows the response is grounded in something live, not stale.

**Key characteristics:**

- **Answer-time, not ingest-time.** Tools fire on the user's question. Nothing is cached or pre-indexed.
- **Manifest-driven.** Each external system is described by a YAML manifest. No Rust code is required to add a new one.
- **Open protocol.** DocBrain speaks [Model Context Protocol](https://modelcontextprotocol.io/) (MCP) 2024-11-05 Streamable HTTP. Any MCP-compliant server works.
- **Off by default.** When `MCP_TOOLS_ENABLED=false` (the default), the synthesis path is byte-identical to the pre-MCP path: no orchestrator, no fast-LLM dispatch, no measurable overhead.
- **Every surface.** Tools dispatch uniformly wherever you ask — the web UI, Slack, and the `docbrain ask "..."` CLI all run the same orchestrator. (CLI questions previously skipped tool dispatch; they no longer do.)
- **Intent-based, not keyword-based.** The tool picker infers intent from the *meaning* of the question, not from keywords. "Any active incidents?" will search the relevant connected tools without the user ever naming "Jira" or "Confluence".

!!! info "Live tools vs. external connectors"
    [External Connectors](connectors.md) pull documents *into* DocBrain on a cron schedule — slow, batched, and indexed. MCP tools query systems *at answer time* — fast, on-demand, and never indexed. Use connectors for narrative knowledge (wiki pages, runbooks). Use MCP tools for state that changes minute-to-minute (ticket status, alert counts, build state).

---

## The two-manifest model for Jira

DocBrain ships two Jira backends in v1, and both load by default. The dispatcher picks between them per question.

### `jira` — Atlassian Remote MCP Server

Atlassian's [hosted Remote MCP Server](https://support.atlassian.com/rovo/docs/getting-started-with-the-atlassian-remote-mcp-server/) exposes the **Teamwork Graph** — a unified view of Jira, Confluence, Compass, and other Atlassian products with their cross-product relationships. This is the right backend for questions like *"what's blocking PROJ-123 across Jira and Confluence?"* or *"which tickets reference this Confluence page?"*

!!! warning "Early-stage rollout"
    Atlassian's Remote MCP Server is currently in early-stage rollout. Live verification shows it returning generic *"We are having trouble completing this action"* errors for queries that work fine via Atlassian's standard REST API. Atlassian is iterating; the `jira_rest` shim is the workaround until parity is reached.

### `jira_rest` — DocBrain-hosted REST shim

A second MCP server is served by `docbrain-server` itself at `/internal/mcp/jira-rest`. It wraps Atlassian's standard REST v3 API (`https://<your-domain>.atlassian.net/rest/api/3/*`) and exposes the same MCP protocol the orchestrator uses for the hosted server — the dispatcher cannot tell them apart at the protocol layer.

This is the **primary path for direct ticket lookups** today: status, assignee, JQL search. It's faster, doesn't depend on Atlassian's MCP rollout, and uses the same `JIRA_API_TOKEN` your existing Jira ingest already needs.

### Who chooses?

**Not the operator.** Both manifests load at boot and are eligible for every question. The fast-LLM dispatcher picks per question:

- "What's the status of PROJ-123?" → `jira_rest.get_issue` (single ticket, REST is faster).
- "What's blocking PROJ-123 across all our tools?" → `jira.get_teamwork_graph_context` (needs cross-product relationships).
- "All open tickets assigned to me" → `jira_rest.search` (JQL is mature, REST is reliable).

If a tool times out or errors, the orchestrator records the failure in the audit log and synthesis proceeds with whatever data is available — including from the other Jira backend if it also fired.

---

## How dispatch works

```
  User question
        │
        ▼
  ┌─────────────────────┐
  │  Fast LLM picks     │   eligibility filter applied first:
  │  tool(s) (parallel) │   user, manifest, auth-mode must be enabled
  └─────────────────────┘
        │
        ▼
  ┌─────────────────────┐
  │  Gateway dispatches │   8s wall-clock budget across the whole fan-out
  │  in parallel        │   each tool has its own latency_budget_ms (7s default)
  └─────────────────────┘
        │
        ▼
  ┌─────────────────────┐
  │  External MCP       │   Atlassian Remote MCP, DocBrain's REST shim,
  │  server(s)          │   GitHub, Sentry, anything MCP-compliant
  └─────────────────────┘
        │
        ▼
  ┌─────────────────────┐
  │  Tool output →      │   each tool result wrapped as a <live_data>
  │  <live_data> block  │   block, prepended to the synthesis prompt
  └─────────────────────┘
        │
        ▼
  ┌─────────────────────┐
  │  Synthesis LLM      │   LLM grounds the answer in retrieved chunks
  │  grounds answer     │   + live data, cites both
  └─────────────────────┘
        │
        ▼
  ┌─────────────────────┐
  │  UI chip + cited    │   chip surfaces "live: jira", "live: github",
  │  answer             │   timeout / error states shown explicitly
  └─────────────────────┘
```

**Three guarantees** the orchestrator enforces:

1. **8-second wall-clock budget** for the entire orchestration step. If a tool exceeds its individual `latency_budget_ms` (7s default), or the total budget runs out, the tool's slot in the prompt shows `timeout` and synthesis proceeds without it.
2. **Audit log** records every dispatch: which manifest, which tool, args (PII-scrubbed), latency, outcome, byte count. Visible to admins per-manifest at `/admin/tools/<id>/audit`.
3. **Eligibility model.** Admins control who can use what. Eligibility is checked on the (principal, manifest, auth_mode) triple before the fast LLM ever sees the question — a user who isn't enabled for `jira` won't have it offered as a candidate.

---

## Hosted vs self-hosted manifests

DocBrain manifests fall into two operational categories. The admin UI at `/admin/tools` labels each manifest with a **Hosted** or **Self** badge so operators can tell at a glance who runs the MCP server.

**Hosted** — the upstream MCP server is operated by the vendor:

- `atlassian` (`mcp.atlassian.com/v1/mcp/authv2`) — Teamwork Graph; covers Jira issues, Confluence pages, Projects, Goals via a single endpoint.
- `slack` (`mcp.slack.com`) — Slack's hosted MCP server; search across messages, files, channels, users.
- `github` (`api.githubcopilot.com/mcp`) — GitHub's hosted MCP server; code, PRs, issues, file contents.

For hosted manifests, DocBrain forwards each dispatch to the vendor's endpoint with the calling user's OAuth token. The vendor's server handles dispatch and enforces the user's real permissions on their side. Failures (auth errors, rate limits) show in the audit log with the upstream error body verbatim.

**Self** — the MCP server runs in-process inside the DocBrain pod, as a loopback shim. The shim wraps a vendor REST API as an MCP server:

- `jira_rest` — wraps Jira Cloud REST API (`/rest/api/3/search`, `/rest/api/3/issue/{key}`). JQL search + ticket fetch by key.
- `confluence_rest` — wraps Confluence Cloud REST API (`/rest/api/content/search`, `/rest/api/content/{id}`). CQL search + page fetch.
- `slack_rest` — wraps Slack Web API (`/api/conversations.replies`, `/api/conversations.history`). Direct thread + channel reads (vendor's hosted MCP doesn't expose these).

For self-hosted manifests, the gateway dispatches via service-account auth (an internal bearer matched on the loopback hop), and the shim itself calls the vendor REST API using credentials drawn from this deployment's env vars (or, for slack_rest, the user's stored OAuth token resolved internally — see the `shim_internal` section below).

**Naming convention:** `<vendor>` = hosted, `<vendor>_rest` = self-hosted shim. Two manifests for the same vendor are sometimes coexistent (e.g. `slack` + `slack_rest`) because the hosted server doesn't expose every operation we need — DocBrain registers both so the picker can route each question type to the right one.

**Operationally, the hosted/self distinction matters because:**

1. **Hosted manifests fail-mode differently** — Atlassian's hosted endpoint returning *"We are having trouble completing this action"* is an upstream vendor issue; self-hosted shims either succeed or fail at our own code boundary with a precise error.
2. **Scope semantics differ** — hosted manifest OAuth scopes are granted to the *vendor's MCP server*; self-hosted shim OAuth scopes (the `shim_internal: true` case) are granted to the *shim* which then calls the vendor's regular API.
3. **Egress allowlists differ** — hosted manifests need outbound HTTPS to the vendor's MCP host; self-hosted shims need outbound HTTPS to the vendor's regular API host. Both are declared in the manifest's `egress_hosts`.

---

## Auth modes

Each manifest declares one or more `modes` under `auth:`. The orchestrator picks the most specific mode the user is eligible for.

### Service-account

A single shared bearer token, read from an env var, used for every request to the upstream MCP server.

```yaml
auth:
  modes:
    - service_account
  service_account:
    secret_refs:
      - JIRA_API_TOKEN
    header_template: "Bearer ${JIRA_API_TOKEN}"
```

**When to use:** self-hosted MCP servers you control (the `jira_rest` shim is the canonical example), single-tenant deployments, or as the fallback when OAuth isn't yet configured. Simplest to operate; no per-user grant flow.

**Trade-off:** every tool call uses the same identity upstream. The external system can't enforce per-user permissions, so you must rely on DocBrain's RBAC + ACL layer for access control.

#### Scoping self-referential queries (`identity_arg`)

Because a service-account tool runs on shared credentials, a first-person question like *"what am I working on?"* has nothing to tie the result to the asking user — the only thing that scopes it is whatever person reference ends up in the query (a JQL `assignee`, a Slack `user`, etc.). If a tool leaves that to the language model, one user's *"my tickets"* can return another user's data.

To make this safe, declare which argument carries the caller's identity and how to write it:

```yaml
tools:
  - name: jira_rest.search
    # ...
    identity_arg:
      arg: jql            # which argument scopes results to a person
      kind: jql_assignee  # how the caller's identity is written into it
```

For a first-person-singular query, DocBrain forces the verified caller's identity into that argument — overriding whatever the model produced — across **every** service-account tool, not just Jira. Supported `kind` values:

| `kind`            | Behavior                                                                          | Example tool          |
|-------------------|-----------------------------------------------------------------------------------|-----------------------|
| `jql_assignee`    | Rewrites the `assignee` clause of a JQL string to the caller                      | Jira search           |
| `cql_creator`     | Rewrites the `creator` clause of a CQL string to the caller                       | Confluence search     |
| `literal`         | Replaces the whole argument value with the caller's email/login                   | GitHub `author`       |
| `slack_user_from` | Resolves the caller's **linked Slack user id** and writes it (some upstreams filter by Slack id, not email). If the caller has no linked identity, the tool is dropped and they're prompted to link it. *Not used by the bundled Slack manifest* (that runs per-user OAuth, where the token already scopes results) — provided for service-account tools that filter by Slack id. | service-account tools keyed on Slack id |

Team, project, and named-person queries (*"what is my team working on"*, *"what is Alice working on"*) are left untouched — the shared service account is designed to see them.

**Fail-closed default:** a service-account tool that does **not** declare `identity_arg` will *refuse* first-person queries rather than risk returning the wrong person's data. So a newly added third-party tool is safe by default; declaring `identity_arg` is how you opt it into answering *"my X"* safely. The named `arg` must be a property in the tool's `args_schema`, or the manifest fails to load.

### OAuth (per-user)

Each user clicks **Connect** on `/integrations` and grants their own token to the external system. Tokens are stored encrypted at rest (AES-256-GCM with `MCP_OAUTH_ENCRYPTION_KEY`) in the `mcp_oauth_tokens` table and refreshed automatically before expiry.

```yaml
auth:
  modes:
    - service_account
    - oauth
  oauth:
    provider: atlassian
    authorize_url: "https://auth.atlassian.com/authorize"
    token_url: "https://auth.atlassian.com/oauth/token"
    scopes:
      # Jira platform REST API
      - "read:jira-work"
      - "read:jira-user"
      # User identity API — required to resolve other users by email
      - "read:me"
      - "read:account"
      # Confluence API — required for cross-product graph traversal
      - "read:confluence-content.all"
      - "read:confluence-content.summary"
      - "read:confluence-space.summary"
      - "read:confluence-user"
      - "search:confluence"
      # Refresh tokens
      - "offline_access"
    client_id_secret_ref: ATLASSIAN_OAUTH_CLIENT_ID
    client_secret_ref: ATLASSIAN_OAUTH_CLIENT_SECRET
    use_pkce: true
```

**When to use:** production multi-user deployments. Each Jira query runs as the asking user — Atlassian enforces their real permissions, and audit trails on the upstream side show who actually asked.

!!! tip "Always include `offline_access`"
    Without the `offline_access` scope, Atlassian (and most OAuth providers) issues access tokens that expire in ~1 hour and no refresh token. Users would have to re-click Connect every hour. The reference `jira` manifest includes it; if you author a new OAuth manifest, copy that pattern.

!!! warning "Scope traps when registering the Atlassian app"
    Atlassian's Developer Console splits scopes across **three separate APIs** — and missing scopes on any of them produce a generic `"You don't have access ..."` error that looks like a permission denial. To enable the full hosted Atlassian MCP surface (Teamwork Graph traversal), in the Developer Console → **Permissions** page:

    1. **User identity API** — Add scopes `read:me` and `read:account`. Without `read:account`, looking up another user by email (`objectType: "AtlassianUser"` on `get_teamwork_graph_context`) fails with an opaque error.
    2. **Confluence API** — Add `read:confluence-content.all`, `read:confluence-content.summary`, `read:confluence-space.summary`, `read:confluence-user`, `search:confluence`. Both `.all` and `.summary` are required — they are independent gates, not a hierarchy.
    3. **Jira API** — `read:jira-work` and `read:jira-user`. Beyond these two, the modern Jira REST API does NOT expose a separate Teamwork Graph scope on classic 3LO apps — traversal is gated on having the underlying read scopes across Jira + Confluence + User Identity.

    All scopes above are read-only. **Do NOT add** anything starting with `write:`, `delete:`, `manage:`, or `admin:` — DocBrain's MCP layer is read-only by design (D1 invariant) and would refuse to dispatch a write tool even if one were granted, but extra scopes widen the blast radius if the OAuth token is ever stolen.

    **Existing connected users must reconnect after a scope change.** Adding scopes to the app does not update tokens already in `mcp_oauth_tokens`. Either ask each user to click **Disconnect** → **Connect** on `/integrations`, or as admin delete the relevant rows from `mcp_oauth_tokens` to force a fresh OAuth dance on next dispatch.

The orchestrator picks OAuth when the requesting user has a stored token for that manifest; otherwise it falls back to service-account if the manifest declares it.

#### Health checks for OAuth-only manifests

A manifest whose only auth mode is `oauth` has no credential of its own, so
the admin **Run health check** and the background health probe can only run
as a user who has connected. Declare that explicitly:

```yaml
auth:
  modes:
    - oauth
  health_check:
    kind: per_user_oauth
  oauth:
    ...
```

The probe then runs as the manifest's **designated probe user** — the first
user to connect is auto-designated, and an admin can change it from the
manifest detail page or via `PUT /api/v1/admin/mcp/manifests/<id>/probe-user`.
Without a probe user the health check reports a permanent failure that names
this fix; without the `health_check` block the probe falls back to a
service-account-first path that has no identity to run as. The shipped
`slack.yaml` carries the block.

### OAuth-for-shim-internal token grants (`shim_internal`)

A small number of manifests use OAuth NOT as a dispatch mode but to grant the user's token to an **in-process shim** which calls the upstream itself. The canonical example is `slack_rest`: the gateway → shim hop runs over loopback using a shared service-account bearer (the shim's auth gate), and the shim then resolves the user's Slack OAuth token from `mcp_oauth_tokens` to call `slack.com/api/conversations.replies` as that user.

For this pattern, set `auth.oauth.shim_internal: true`:

```yaml
auth:
  modes:
    - service_account   # the only DISPATCH mode the gateway will use
    - oauth             # required so /oauth/mcp/init lets users connect
  service_account:
    secret_refs: [DOCBRAIN_INTERNAL_MCP_SECRET]
    header_template: "Bearer ${DOCBRAIN_INTERNAL_MCP_SECRET}"
  oauth:
    provider: generic
    authorize_url: "https://upstream.example.com/oauth/authorize"
    token_url:     "https://upstream.example.com/oauth/token"
    scopes:        ["read:something"]
    client_id_secret_ref: UPSTREAM_OAUTH_CLIENT_ID
    client_secret_ref:    UPSTREAM_OAUTH_CLIENT_SECRET
    use_pkce: true
    shim_internal: true   # <-- gateway never dispatches OAuth for this manifest
```

What each piece does:

- **`modes: [service_account, oauth]`** — both modes are declared so the install path auto-seeds enablement rows for both, AND the `/oauth/mcp/init` enablement gate finds an `oauth`-mode row when the user clicks Connect on `/integrations`. Without `oauth` in `modes`, the Connect button silently 403s with *"no OAuth enablement for this manifest"*.
- **`oauth.shim_internal: true`** — the picker reads this flag (`registry/manifest.rs`) and refuses to ever select OAuth as the dispatch mode. Dispatch always goes via service-account, sending the SA bearer plus an `X-Docbrain-User-Id` header to the shim. The shim then looks up the user's OAuth token internally.

**When NOT to use `shim_internal`**: regular OAuth manifests (`jira`, `slack`, etc.) where the gateway calls the upstream directly with the user's token. For those, leave `shim_internal` unset (defaults to `false`). The picker will then prefer OAuth-mode dispatch when the user has a valid token, falling back to service-account otherwise.

**Rule of thumb:**

| Question | Answer | Then `shim_internal` is… |
|---|---|---|
| Does the gateway call the real upstream directly with the user's OAuth token? | Yes | `false` (default) |
| Does the gateway call a loopback shim that internally resolves the user's token from `mcp_oauth_tokens`? | Yes | `true` |

**Picker behaviour, exhaustively:**

| `modes` | `shim_internal` | User has OAuth token | Picker selects | What gets sent to upstream |
|---|---|---|---|---|
| `[oauth]` | `false` | yes | OAuth | User's token in `Authorization: Bearer` |
| `[oauth]` | `false` | no | — | Not dispatched; UI shows "Connect" |
| `[oauth]` | `true` | any | — | Picker skips OAuth, no SA available → manifest unusable. **Misconfiguration.** |
| `[sa]` | n/a | n/a | SA | SA bearer in `Authorization: Bearer` |
| `[sa, oauth]` | `false` | yes | OAuth | User's token in `Authorization: Bearer` (upstream sees user identity) |
| `[sa, oauth]` | `false` | no | SA | SA bearer (upstream sees shared identity) |
| `[sa, oauth]` | `true` | yes | SA | SA bearer + `X-Docbrain-User-Id`; shim looks up user's token internally |
| `[sa, oauth]` | `true` | no | SA | SA bearer + `X-Docbrain-User-Id`; shim fails to find token, returns "Connect" error |

**Symptom of getting this wrong:**

- `shim_internal: true` but you actually wanted OAuth dispatch → upstream gets your SA bearer instead of the user's token. Either the upstream rejects (401) or the call runs as the SA identity instead of the user (results scoped wrong).
- `shim_internal: false` (or unset) on an in-process shim that expects the SA bearer → the gateway sends the user's OAuth token to a loopback endpoint that only accepts the internal SA bearer; the shim returns 401 with *"missing Mcp-Session-Id"*. This was the exact slack_rest dispatch bug on 2026-05-26.

---

## Enabling MCP tools in production

The full env-var and Helm-value reference lives in [Configuration → MCP Tool Platform](configuration.md#mcp-tool-platform). Here's the operator checklist:

### 1. Generate the encryption key

```bash
openssl rand -base64 32
```

Store the output as `MCP_OAUTH_ENCRYPTION_KEY`. Loss of this key means every per-user OAuth token in `mcp_oauth_tokens` becomes unreadable and every user has to reconnect. Treat it like a database master key.

### 2. Set Helm values

```yaml
mcpTools:
  enabled: true
  encryptionKey: ""              # MCP_OAUTH_ENCRYPTION_KEY (from secret)
  internalShimSecret: ""         # DOCBRAIN_INTERNAL_MCP_SECRET (from secret)
  manifestDir: /etc/docbrain/mcp-manifests

  serviceAccount:
    jira:
      apiToken: ""               # JIRA_API_TOKEN (from secret)
      cloudId: ""                # JIRA_CLOUD_ID (workspace UUID)

  oauth:
    atlassian:
      clientId: ""               # ATLASSIAN_OAUTH_CLIENT_ID (from secret)
      clientSecret: ""           # ATLASSIAN_OAUTH_CLIENT_SECRET (from secret)
```

In production, leave the string fields empty in `values.yaml` and inject them via `existingSecret`. See [Configuration → MCP Tool Platform](configuration.md#mcp-tool-platform) for the full env-var table.

### 3. Register the OAuth client with your IdP

For Atlassian: **Developer Console → OAuth 2.0 (3LO) → Create app**. Set the callback URL to:

```
https://<your-domain>/api/v1/oauth/mcp/callback/jira
```

The path segment after `callback/` is the manifest id (`jira`). If you add a manifest with a different id, the callback path changes accordingly.

### 4. Helm upgrade

```bash
helm upgrade docbrain ./helm/docbrain \
  -f values.yaml \
  --namespace docbrain
kubectl rollout status deploy/docbrain-server --namespace docbrain --timeout=5m
```

Verify the orchestrator wired up:

```bash
kubectl logs deploy/docbrain-server --namespace docbrain --since=2m | grep -i mcp
# Expected:
#   "MCP_TOOLS_ENABLED=true; constructing orchestrator..."
#   "Loaded N MCP manifest(s) from /etc/docbrain/mcp-manifests"
#   "MCP orchestrator: enabled"
```

### 5. Enable per principal

An admin visits `/admin/tools/<manifest-id>/enablements` and grants eligibility to user groups or individual users. Until a principal is enabled, the manifest exists but the orchestrator won't offer it to that user's questions.

### 6. Users connect

Eligible users see a **Connect** button for that manifest on `/integrations`. They click through the OAuth flow; the token is encrypted and persisted; subsequent `/ask` calls dispatch as that user.

!!! warning "Some upstreams require org-admin enablement for API-token auth"
    Atlassian's API-token mode requires an org admin to enable MCP-style programmatic access at the workspace level. If users see "permission" errors on the chip even though the token is valid, the OAuth path is the answer — OAuth grants work without the org-level toggle.

---

## Admin UI

### `/admin/tools` — catalog

The catalog lists every installed manifest with:

- Display name, category, icon
- Tool count
- Enablement count (principals + groups eligible)
- Declared auth modes (`service_account`, `oauth`, or both)
- Health badge (last test-connection outcome)

### `/admin/tools/<id>` — manifest detail

Per-manifest detail page tabs:

- **Tools.** Each tool's name, description, JSON Schema for arguments, `latency_budget_ms`, and `output_size_cap_bytes`.
- **Eligibility.** Add or remove principals (users, SSO groups) and auth modes. All mutations audit-logged.
- **Test Connection.** Sends an MCP `initialize` followed by `tools/list` against the configured endpoint with the current service-account credentials. Surfaces the upstream's reported protocol version and tool list. Use this after rotating secrets or changing the manifest.
- **Audit Log.** Filtered to this manifest. Shows dispatch events (who, when, args, latency, outcome) and admin events (enablement changes, secret rotations).

All mutating actions across this UI are written to the audit log with the acting admin's principal.

---

## Adding a new MCP manifest

The platform is **manifest-driven** — adding GitHub, Sentry, Datadog, PagerDuty, or anything else MCP-compliant requires no Rust code.

### Step 1 — Author the manifest

Drop a new YAML file under `config/mcp-manifests/<id>.yaml`. Use the shipped `jira.yaml` and `jira-rest.yaml` as references — both demonstrate the full manifest schema.

```yaml
manifest_version: 1
id: github
display_name: "GitHub"
description: "Live GitHub PR / issue / repo metadata via the official MCP server."
category: "scm"

server:
  transport: http_sse
  endpoint: "https://api.githubcopilot.com/mcp"
  protocol_version: "2024-11-05"

auth:
  modes:
    - oauth
  oauth:
    provider: github
    authorize_url: "https://github.com/login/oauth/authorize"
    token_url: "https://github.com/login/oauth/access_token"
    scopes: ["repo", "read:org"]
    client_id_secret_ref: GITHUB_MCP_CLIENT_ID
    client_secret_ref: GITHUB_MCP_CLIENT_SECRET
    use_pkce: true

tools:
  - name: github.get_pull_request
    upstream_name: get_pull_request
    description: "Fetch a single PR by owner/repo/number..."
    args_schema:
      type: object
      properties:
        owner: { type: string }
        repo: { type: string }
        pull_number: { type: integer }
      required: [owner, repo, pull_number]
    output_size_cap_bytes: 16384
    latency_budget_ms: 7000

egress:
  hosts:
    - "api.githubcopilot.com"
    - "github.com"

rbac:
  required_role: viewer

retention:
  audit_log_days: 90
```

### Step 2 — Symlink into the Helm chart

The chart ships its own `files/mcp-manifests/` directory that becomes the mounted ConfigMap. For Helm-packaged deployments, symlink your new manifest in:

```bash
cd helm/docbrain/files/mcp-manifests
ln -s ../../../../config/mcp-manifests/github.yaml github.yaml
```

The relative path keeps the chart and the config repo in sync — the manifest is a single source of truth.

### Step 3 — Restart the server

Manifests are loaded once at boot. Restart `docbrain-server` (or `helm upgrade` to redeploy the ConfigMap and trigger a rollout).

### Step 4 — Admin enables eligibility

Visit `/admin/tools/github/enablements` and grant access. Users now see **Connect** on `/integrations`.

That's the entire flow. No Rust code, no migrations, no deployment beyond the rollout.

!!! info "Schema validation at boot"
    The manifest loader validates every YAML file at startup. A malformed manifest is logged with the parse error and skipped — the server still starts with the remaining valid manifests. Watch the boot log after dropping in a new file. A manifest whose `server.endpoint` references an env var that is **unset** is also skipped (logged as "failed to materialize during bootstrap — skipped") rather than disabling the whole MCP platform — so an unconfigured optional integration never takes the others down.

---

## Adding Slack search

Slack search uses Slack's **official hosted MCP server** (`mcp.slack.com`) — the same model as the hosted Jira manifest. DocBrain points at Slack's URL; each user connects via OAuth on `/integrations`, and their search runs as themselves (scoped to the channels they're in plus all public channels). There is **no self-hosted server and no shared token** — you register a Slack OAuth app, set two secrets, and connect.

The `slack` manifest is bundled with DocBrain — you do not author it.

!!! info "Read-only by design"
    Slack's hosted server exposes search/read tools **and** write tools (send message, manage canvases). DocBrain imports the Slack catalog via [dynamic discovery](#dynamic-tool-discovery) and applies the read-only gate (`annotations.readOnlyHint == true`), so **only the search/read tools are registered — the write tools are dropped automatically.** DocBrain never sends Slack messages.

### Step 1 — Register a Slack OAuth app

Create (or extend) a Slack app for OAuth with the **user-token** search scopes:

- `search:read.public`, `search:read.private`, `search:read.mpim`, `search:read.im`, `search:read.files`

Slack uses granular per-conversation-type scopes — there is no single `search:read`. Note these are **user-token** scopes (the `oauth/v2_user/authorize` flow), distinct from a bot token; a `xoxb-` bot token cannot search. Grab the app's **client id** and **client secret**.

### Step 2 — Set DocBrain's Slack config

Only the OAuth app credentials are needed (plus the MCP OAuth encryption key, required for any OAuth manifest):

```yaml
mcpTools:
  enabled: true
  encryptionKey: ""        # MCP_OAUTH_ENCRYPTION_KEY — required for OAuth manifests
  oauth:
    slack:
      clientId: ""         # Slack OAuth app client id
      clientSecret: ""     # Slack OAuth app client secret
```

If you use an externally-managed secret (`existingSecret`), add `SLACK_OAUTH_CLIENT_ID` and `SLACK_OAUTH_CLIENT_SECRET` to it instead.

### Step 3 — Restart and enable

Restart the server. The boot log shows the `slack` manifest loading and dynamic discovery probing `mcp.slack.com` (importing only the read/search tools). Then enable it per principal at `/admin/tools/slack/enablements`, exactly like any other tool.

### Step 4 — Users connect

Each user clicks **Connect** on `/integrations` and authorizes Slack. Their search then runs as them — "what was discussed about the deploy in #eng?" or "my Slack messages" — scoped by Slack to the channels they belong to and all public channels. Because each user's own token scopes results, no identity rewriting is needed.

---

## Dynamic tool discovery

Hand-declaring every tool in `tools:` is fine for small, stable MCP servers — but upstreams like GitHub, Sentry, or Datadog ship dozens of tools and add new ones between DocBrain releases. For those, a manifest can opt into **dynamic discovery**: at boot (and on a refresh interval), DocBrain queries the upstream's `tools/list` and auto-populates the catalog.

A dynamic manifest declares `tool_discovery.mode: dynamic` and may leave `tools: []`. Static and discovered tools can coexist; when names collide, the static entry wins only if it sets `override_discovered: true`, otherwise both are dropped and the manifest is marked `degraded_collisions` until the conflict is resolved.

**Read-only invariant.** DocBrain only registers discovered tools that the upstream marks `annotations.readOnlyHint == true`. The same gate applies to static tools via a required `read_only` field. This is a platform-wide guarantee: DocBrain does not dispatch write operations through MCP, ever.

As **defense-in-depth** (in case an upstream mislabels a write tool as read-only), discovery also drops any tool whose **name implies a mutation** — independent of `readOnlyHint`. A conservative, token-boundary verb check (`send`, `create`, `update`, `delete`, `post`, … matched as whole tokens, so `created_by`/`settings` are not false positives; camelCase like `sendMessage` is handled) blocks the tool and increments `mcp_discovery_write_name_blocked`. Reads (`get`, `list`, `search`, `read`, …) pass through.

**OAuth-only manifests need a probe user.** Because periodic `tools/list` calls need credentials, an admin must designate a user whose OAuth token the discovery worker borrows. Until designated, the manifest's status sits at `requires_probe_user` and no tools are served. Mixed-auth manifests fall back to the service-account header for probes and need no extra setup.

See [Configuration → Dynamic tool discovery](configuration.md#dynamic-tool-discovery) for the full `tool_discovery` YAML block and the [API Reference → Admin — MCP Manifests](api-reference.md#admin--mcp-manifests) for the discovery-probe and probe-user admin endpoints.

---

## Admin-installed manifests (runtime install, no redeploy)

In addition to manifests committed to git under `config/mcp-manifests/`, an admin can install a manifest at **runtime** through the admin API. This is useful for customer-bespoke internal MCPs — a private status page, an internal Jira-clone, a proprietary observability tool — that shouldn't live in a public manifest repo and shouldn't require a DocBrain release to add.

Runtime-installed manifests use the same orchestrator, the same gateway, the same audit log, and the same OAuth flow as git-tracked manifests. The only differences:

- **Source attribution.** Audit rows stamp `manifest_source = 'tier2'` (vs `'tier1'` for git) and `manifest_version` (the install version number). Compliance teams can reconstruct exactly which manifest schema was active for any historical dispatch.
- **Versioned with rollback.** Every install increments a per-manifest version. To roll back after a bad install, the admin POSTs the prior version number to the `/activate` endpoint — a one-row UPDATE, atomic, audit-clean.
- **Per-secret storage choice.** Each secret is either **inline** (admin pastes the value, server encrypts it with AES-256-GCM into `mcp_manifest_secrets.ciphertext`) or **env** (admin references a pre-mounted env var by name). Inline mode requires `DOCBRAIN_MCP_SECRET_KEY` set at boot; env mode works without it.
- **Precedence policy.** When the same `manifest_id` exists in both sources, a `manifest_resolution_policy` DB row picks the winner. Default is `['tier1', 'tier2']` (git wins). For incident response, an operator can pin a manifest_id to a single source without uninstalling.

### Install a manifest

`POST /api/v1/admin/mcp/manifests` accepts either a `yaml` body (paste) or a `url` reference (server-side fetch, HTTPS-only, host-allowlisted, no redirects, 5s timeout, 64KB cap). Secrets are submitted alongside the manifest:

```bash
curl -X POST https://docbrain.example.com/api/v1/admin/mcp/manifests \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "yaml": "<manifest body>",
    "secrets": [
      {"key": "api_token", "mode": "inline", "value": "<bearer>"}
    ],
    "description": "Internal status page MCP"
  }'
```

Returns `{"manifest_id": "<id>", "version": 1}`. The manifest is dispatchable within ~1 second (a `LISTEN/NOTIFY` channel wakes the resolver).

### Rollback after a bad install

```bash
# List versions
curl https://.../api/v1/admin/mcp/manifests/<id>/versions \
  -H "Authorization: Bearer $ADMIN_TOKEN"

# Activate a prior version
curl -X POST https://.../api/v1/admin/mcp/manifests/<id>/activate \
  -H "Authorization: Bearer $ADMIN_TOKEN" \
  -d '{"version": 3}'
```

### Manifest detail (no secret values exposed)

```bash
curl https://.../api/v1/admin/mcp/manifests/<id> \
  -H "Authorization: Bearer $ADMIN_TOKEN"
```

Returns `{manifest_id, active_version, secrets: [{key, mode, env_var_name?, rotated_at?}]}`. Ciphertext is never included in the response.

### Configuration

The two settings relevant to admin install live under `tier2:` in `config/default.yaml`:

```yaml
tier2:
  fetch_allowed_hosts: []      # empty = URL install disabled; paste only
  max_manifest_bytes: 65536    # 64 KB hard cap for URL fetches
```

The optional master encryption key is sourced from the `DOCBRAIN_MCP_SECRET_KEY` env var (base64-encoded 32 bytes). Without it, inline-mode secrets fail closed with 503 — env-mode secrets and Tier 1 manifests continue to work.

!!! note "Day-to-day operator commands"
    Full operational procedures — secret rotation, incident-time source pinning, audit-replay queries — live in the operator runbook shipped with the source tree.

---

## Limitations and known issues

### Atlassian Remote MCP rollout is early-stage

Atlassian's hosted MCP server occasionally returns *"We are having trouble completing this action"* errors for queries that work fine via REST. The `jira_rest` shim is the workaround — both manifests ship by default, the dispatcher routes appropriately, and Atlassian is iterating on the hosted server. As Atlassian stabilizes, the `jira` manifest will become the primary path; `jira_rest` will remain as a fast fallback.

### API-token mode may require org-admin enablement

Some MCP servers' API-token authentication requires an organization administrator to enable programmatic MCP access at the workspace level (Atlassian's case). If users see permission errors on the chip despite a valid token, switch the manifest to OAuth — OAuth grants work without the org-level toggle.

### Per-tool latency budget vs. orchestrator budget

- Each tool has `latency_budget_ms` (default `7000`).
- The orchestrator has an 8-second total wall-clock budget across the entire fan-out.
- The per-tool budget is the **single source of truth** for in-process REST shims (`jira-rest`, `confluence-rest`, `slack-rest`). The gateway injects the budget on every dispatch via an internal `X-DocBrain-Tool-Budget-Ms` header; the shim uses it as its upstream-call timeout. To change a timeout, edit the manifest — no code change required.

A tool that takes longer than its own budget shows `timeout` on the chip. If you're seeing frequent timeouts on a specific tool, tune `latency_budget_ms` in the manifest — but staying under the 8-second orchestrator ceiling is what keeps `/ask` from feeling hung.

**Security note for the budget header.** The header is trusted only because the request already traversed the shim's loopback gate (`127.0.0.1` only) and constant-time bearer compare. The shim additionally clamps the header value to `[1000, 30000]` ms regardless of what it received, so a forged or misbehaving value cannot pin a worker.

### Egress allow-list is enforced

Every manifest declares `egress.hosts`. The orchestrator rejects upstream URLs that don't match. This is intentional — a malicious manifest can't be used to probe arbitrary internal services. If you add a manifest whose upstream lives at a new domain, declare it in `egress.hosts` or the requests will be blocked at dispatch time.

---

## Related

- **Configuration:** [MCP Tool Platform env vars and Helm values](configuration.md#mcp-tool-platform)
- **Deployment:** [Kubernetes](kubernetes.md)
- **External Connectors:** [Build adapters for any knowledge source](connectors.md) — note: MCP tools are *answer-time live data*, distinct from ingestion-time connectors.
- **Runbook:** [MCP Shadow Run](runbooks/mcp-shadow-run.md) — capture an answer-quality baseline, flip the switch, and verify pass count holds before retiring legacy enrichment paths.
