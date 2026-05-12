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
      - "read:jira-work"
      - "read:jira-user"
      - "offline_access"     # required for refresh tokens
    client_id_secret_ref: ATLASSIAN_OAUTH_CLIENT_ID
    client_secret_ref: ATLASSIAN_OAUTH_CLIENT_SECRET
    use_pkce: true
```

**When to use:** production multi-user deployments. Each Jira query runs as the asking user — Atlassian enforces their real permissions, and audit trails on the upstream side show who actually asked.

!!! tip "Always include `offline_access`"
    Without the `offline_access` scope, Atlassian (and most OAuth providers) issues access tokens that expire in ~1 hour and no refresh token. Users would have to re-click Connect every hour. The reference `jira` manifest includes it; if you author a new OAuth manifest, copy that pattern.

The orchestrator picks OAuth when the requesting user has a stored token for that manifest; otherwise it falls back to service-account if the manifest declares it.

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
    The manifest loader validates every YAML file at startup. A malformed manifest is logged with the parse error and skipped — the server still starts with the remaining valid manifests. Watch the boot log after dropping in a new file.

---

## Limitations and known issues

### Atlassian Remote MCP rollout is early-stage

Atlassian's hosted MCP server occasionally returns *"We are having trouble completing this action"* errors for queries that work fine via REST. The `jira_rest` shim is the workaround — both manifests ship by default, the dispatcher routes appropriately, and Atlassian is iterating on the hosted server. As Atlassian stabilizes, the `jira` manifest will become the primary path; `jira_rest` will remain as a fast fallback.

### API-token mode may require org-admin enablement

Some MCP servers' API-token authentication requires an organization administrator to enable programmatic MCP access at the workspace level (Atlassian's case). If users see permission errors on the chip despite a valid token, switch the manifest to OAuth — OAuth grants work without the org-level toggle.

### Per-tool latency budget vs. orchestrator budget

- Each tool has `latency_budget_ms` (default `7000`).
- The orchestrator has an 8-second total wall-clock budget across the entire fan-out.

A tool that takes longer than its own budget shows `timeout` on the chip. If you're seeing frequent timeouts on a specific tool, tune `latency_budget_ms` in the manifest — but staying under the 8-second orchestrator ceiling is what keeps `/ask` from feeling hung.

### Egress allow-list is enforced

Every manifest declares `egress.hosts`. The orchestrator rejects upstream URLs that don't match. This is intentional — a malicious manifest can't be used to probe arbitrary internal services. If you add a manifest whose upstream lives at a new domain, declare it in `egress.hosts` or the requests will be blocked at dispatch time.

---

## Related

- **Configuration:** [MCP Tool Platform env vars and Helm values](configuration.md#mcp-tool-platform)
- **Deployment:** [Kubernetes](kubernetes.md)
- **External Connectors:** [Build adapters for any knowledge source](connectors.md) — note: MCP tools are *answer-time live data*, distinct from ingestion-time connectors.
- **Runbook:** [MCP Shadow Run](runbooks/mcp-shadow-run.md) — capture an answer-quality baseline, flip the switch, and verify pass count holds before retiring legacy enrichment paths.
