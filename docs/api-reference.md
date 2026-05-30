# API Reference

Base URL: `http://localhost:3000` (default)

Most endpoints require authentication via Bearer token or API key:

```
Authorization: Bearer db_sk_...
```

## OpenAPI Specification

DocBrain auto-generates an OpenAPI 3.1 specification from all API routes. The spec includes request/response schemas, authentication requirements, and error codes for every endpoint.

| Resource | URL |
|----------|-----|
| **Swagger UI** | `GET /api/docs` |
| **OpenAPI JSON** | `GET /api/docs/openapi.json` |

The Swagger UI is publicly accessible (no authentication required) and provides interactive API exploration with "Try it out" functionality. Use the OpenAPI JSON endpoint for client code generation with tools like `openapi-generator`, `oapi-codegen`, or `swagger-codegen`.

The specification is versioned — the `info.version` field tracks the DocBrain release version.

---

## Authentication

### Login

```
POST /api/v1/auth/login
```

Exchange email + password for a session token. The token is a `db_sk_...` API key with a TTL set by `LOGIN_SESSION_TTL_HOURS`.

**Request Body:**
```json
{
  "email": "you@example.com",
  "password": "your-password"
}
```

**Response:**
```json
{
  "token": "db_sk_...",
  "expires_at": "2026-03-28T12:00:00Z"
}
```

---

### Logout

```
POST /api/v1/auth/logout
```

Revokes the current session token. Requires authentication.

**Response:** `200 OK` with `{"ok": true}`

---

### Verify Auth / Whoami

```
GET /api/v1/auth/me
```

Returns the identity of the current token (API key or session key). Useful for verifying an API key is valid.

**Response:**
```json
{
  "key_id": "uuid",
  "name": "Platform Team Key",
  "role": "editor",
  "allowed_spaces": ["PLATFORM", "SRE"],
  "created_at": "2025-12-01T00:00:00Z"
}
```

---

## Core Endpoints

### Health Check

```
GET /api/v1/health
```

Returns `200 OK` with `{"status": "ok"}`. Does not require authentication — used by load balancers and container health probes.

---

### Ask a Question

```
POST /api/v1/ask
```

**Request Body:**
```json
{
  "question": "How do I deploy to production?",
  "session_id": "optional-uuid-for-conversation-continuity",
  "space": "PLATFORM",
  "spaces": ["PLATFORM", "SRE"],
  "stream": true
}
```

- `session_id` — optional UUID to continue a conversation across turns
- `space` — **soft boost**: results from this space get a 1.5× score multiplier but other spaces still appear. Use when you want cross-space results with your team's docs ranked first.
- `spaces` — **hard filter**: only return results from these spaces for this request. If combined with an API key's `allowed_spaces`, the intersection is used (most restrictive wins). Omit to search all spaces.
- `stream` — if `true`, returns SSE; if `false` (default), returns JSON

**Response (non-streaming):**
```json
{
  "answer": "To deploy to production, follow these steps...",
  "sources": [
    {
      "title": "Deployment Guide",
      "heading": "Production Deployment",
      "content": "...",
      "source_url": "https://...",
      "score": 0.92,
      "freshness_score": 82.0,
      "freshness_status": "fresh"
    }
  ],
  "session_id": "uuid",
  "episode_id": "uuid",
  "turn": 1,
  "intent": "procedural",
  "picker_trace_id": "uuid",
  "user_failed_relevant": [],
  "user_unconnected_relevant": ["jira.search"]
}
```

When live MCP tools are involved, the response carries three optional fields (all omitted when empty / not applicable):

- `picker_trace_id` — present when MCP tools fired for this request. Pass it to `GET /api/v1/ask/picker-trace/{request_id}` to retrieve the full tool-selection trace (what was considered, selected, rejected, and why). Powers the "Why these tools?" explainability panel in the UI.
- `user_failed_relevant` — tool names the picker would have used, but the caller's OAuth token for that integration is expired/failed. Drives a "reconnect" prompt in the UI.
- `user_unconnected_relevant` — tool names the picker would have used, but the caller has not connected that integration at all. Drives a "connect this tool" prompt in the UI.

**Streaming Response** (`stream: true`):

Returns Server-Sent Events (SSE):

```
event: phase
data: {"status": "started", "phase": "retrieval", "description": "Searching documents..."}

event: phase
data: {"status": "completed", "phase": "retrieval", "duration_ms": 145, "result_count": 5}

event: token
data: {"text": "To deploy"}

event: token
data: {"text": " to production"}

event: answer
data: {"answer": "...", "sources": [...], "session_id": "...", "episode_id": "...", "intent": "procedural"}
```

### GET /api/v1/ask/picker-trace/{request_id}

Returns the tool-picker trace for a recent `/ask` request — the catalog the
fast LLM saw, the tools it selected and rejected, its 1-sentence rationale,
and the user-specific buckets (`user_unconnected_relevant`,
`user_failed_relevant`) used to surface "connect this tool" hints in the UI.

**Path params:**
- `{request_id}` — the per-request id returned in the `/ask` response.

**RBAC:** any authenticated user. The handler restricts results to the
calling user — cross-user lookups return `404` (existence-disclosure
prevention). Service-account API keys (which have no `user_id`) get `403`.

**Response (200 OK):**
```json
{
  "request_id": "uuid",
  "user_id": "uuid",
  "considered": [
    { "name": "github.issue_read", "manifest_id": "github" }
  ],
  "selected": [
    { "name": "github.issue_read", "args": { "...": "..." } }
  ],
  "rejected": [
    { "name": "jira.search", "reason": "not_relevant_to_query" }
  ],
  "rationale": "Fetching the linked issue gives the deploy-rollback context.",
  "user_unconnected_relevant": [
    { "manifest_id": "slack", "display_name": "Slack" }
  ],
  "user_failed_relevant": []
}
```

**Responses:**
- `200 OK` — trace returned.
- `403 Forbidden` — caller is a service-account API key (no `user_id`).
- `404 Not Found` — `request_id` unknown, expired, or owned by a different user.

Traces are held in an in-process per-user LRU; they expire after roughly one
hour or under cache pressure. The endpoint is read-only and does not write
to `audit_log`.

---

### Submit Feedback

```
POST /api/v1/feedback
```

**Request Body:**
```json
{
  "episode_id": "uuid-from-ask-response",
  "feedback": 1
}
```

`feedback`: `1` (helpful) or `-1` (not helpful). Negative feedback seeds the Autopilot gap detection pipeline.

---

### Freshness Report

```
GET /api/v1/freshness?space=DOCS
GET /api/v1/freshness?tags=architecture,api
GET /api/v1/freshness?archived=true
```

**Query Parameters** (mutually exclusive — `archived` > `tags` > `space`):
- `space` (optional) — Filter by document space
- `tags` (optional) — Comma-separated source labels. Returns docs whose `source_labels` overlap any value (e.g. `?tags=architecture,api`)
- `archived` (optional) — When `true`, returns docs whose `lifecycle_status` is non-active (archived / reference / deprecated). Used to populate the "View excluded (N)" modal in the UI.

**Response:**
```json
{
  "space": "DOCS",
  "summary": { "total_docs": 142, "fresh": 98, "review": 27, "stale": 12, "outdated": 5, "avg_score": 76.3 },
  "documents": [
    {
      "document_id": "123",
      "title": "API Guide",
      "space": "DOCS",
      "source_url": "https://...",
      "total_score": 45.2,
      "status": "stale",
      "source_labels": ["api", "v2"],
      "lifecycle_status": "active",
      "time_decay_score": 30,
      "engagement_score": 50,
      "content_currency_score": 40,
      "link_health_score": 60,
      "contradiction_score": 80
    }
  ]
}
```

---

### Mark a Document Archived (Lifecycle Override)

Manually set a document's lifecycle status. Sticky — survives future syncs even if the source-system label changes. Requires admin.

```
PATCH /api/v1/documents/{id}/lifecycle
Content-Type: application/json

{ "status": "archived" }
```

`status` must be one of: `active`, `archived`, `reference`, `deprecated`. Setting `active` re-enables freshness scoring for the doc and triggers a rescore.

**Response:**
```json
{ "id": "uuid", "lifecycle_status": "archived", "lifecycle_source": "manual" }
```

---

### Backfill Lifecycle Across the Corpus

Re-derive `lifecycle_status` for every auto-managed doc from current `source_labels` and `freshness.exclusion_rules` config. Run after editing exclusion rules. Manual overrides are preserved. Requires admin.

```
POST /api/v1/freshness/backfill-lifecycle
```

**Response:**
```json
{ "changed": 8043, "total_auto_managed": 8874 }
```

---

### Analytics

```
GET /api/v1/analytics?days=30&space=ENG&user_id=uuid
```

**Query Parameters:**
- `days` (optional, default: `30`) — Reporting period in days
- `space` (optional) — Filter most-retrieved docs by Confluence space
- `user_id` (optional) — Filter query statistics by a specific user UUID

**Response:**
```json
{
  "period_days": 30,
  "total_queries": 1247,
  "unique_users": 45,
  "avg_feedback": 0.82,
  "unanswered_rate": 0.12,
  "queries_by_day": [
    {"date": "2025-01-15", "count": 42}
  ],
  "top_intents": [
    {"intent": "procedural", "count": 456}
  ],
  "top_queries": [
    {"query": "How do I deploy?", "count": 18}
  ],
  "most_retrieved_docs": [
    {
      "title": "Deploy Guide",
      "source_url": "https://...",
      "space": "PLATFORM",
      "retrieval_count": 94,
      "freshness_score": 45.0,
      "freshness_status": "stale"
    }
  ]
}
```

**CSV export:**

```
GET /api/v1/analytics/export?days=30
```

Returns a CSV file of all query episodes in the period. Useful for external BI tools.

---

### Analytics CSV Export

```
GET /api/v1/analytics/export?days=30
```

Returns a `.csv` file of all query episodes in the requested period. Columns: `episode_id`, `created_at`, `user_id`, `query`, `intent`, `feedback`, `space`.

---

### Server Configuration

```
GET /api/v1/config
```

No authentication required. Returns enabled features and server version.

```json
{
  "version": "0.6.0",
  "features": {
    "freshness": true,
    "analytics": true,
    "slack": true,
    "autopilot": true,
    "incident_mode": true
  }
}
```

---

### Incident Mode

```
POST /api/v1/incident
```

**Request Body:**
```json
{
  "description": "API latency spike affecting checkout service",
  "severity": "SEV-1"
}
```

Activates incident mode, which prioritizes retrieval of runbooks and incident playbooks.

---

### Admin Dashboard

```
GET /api/v1/dashboard
```

Single-request admin overview. Returns all key health metrics in one parallel-fetched payload — designed for dashboards that need to avoid multiple round-trips.

**Response:**
```json
{
  "health": {
    "total_documents": 342,
    "overall_health_score": 67.3,
    "freshness_distribution": {
      "fresh": 120,
      "review": 89,
      "stale": 72,
      "outdated": 41,
      "archive": 20
    },
    "top_stale_cited_docs": [
      {
        "title": "Deploy Guide",
        "freshness_score": 23.0,
        "citations_last_7d": 47,
        "contradiction_score": 45.0
      }
    ],
    "coverage_gaps": 15
  },
  "autopilot": {
    "total_gaps": 20,
    "open_gaps": 14,
    "critical_gaps": 3,
    "drafts_generated": 8,
    "drafts_published": 2,
    "last_analysis_at": "2025-02-20T14:30:00Z"
  },
  "forecast": {
    "current_open_gaps": 14,
    "projected_new_critical_30d": 5,
    "projected_total_30d": 19,
    "avg_weekly_new_gaps": 2.5,
    "avg_weekly_resolved": 1.0,
    "trend": "worsening"
  },
  "freshness_distribution": {
    "fresh": 120,
    "review": 89,
    "stale": 72,
    "outdated": 41,
    "archive": 20
  },
  "top_gaps": [...],
  "top_docs": [...],
  "recent_audit": [
    {
      "action": "gap_dismissed",
      "entity_id": "uuid",
      "actor": "admin@example.com",
      "created_at": "2026-02-25T10:00:00Z"
    }
  ]
}
```

---

## Autopilot Endpoints

Requires `AUTOPILOT_ENABLED=true`. All endpoints require authentication.

### Autopilot Summary

```
GET /api/v1/autopilot/summary
```

**Response:**
```json
{
  "total_gaps": 20,
  "open_gaps": 14,
  "critical_gaps": 3,
  "drafts_generated": 8,
  "drafts_published": 2,
  "last_analysis_at": "2025-02-20T14:30:00Z"
}
```

---

### Gap Growth Forecast

```
GET /api/v1/autopilot/forecast
```

Returns a 30-day projection of gap cluster growth based on the last 4 weeks of creation and resolution rates (linear extrapolation).

**Response:**
```json
{
  "current_open_gaps": 14,
  "projected_new_critical_30d": 5,
  "projected_total_30d": 19,
  "avg_weekly_new_gaps": 2.5,
  "avg_weekly_resolved": 1.0,
  "trend": "worsening"
}
```

`trend` is one of:
- `"improving"` — resolution rate ≥ 75% of creation rate
- `"stable"` — resolution rate ≥ 40% of creation rate
- `"worsening"` — resolution rate < 40% of creation rate

---

### List Gap Clusters

```
GET /api/v1/autopilot/gaps?limit=20&status=open&severity=critical
```

**Query Parameters:**
- `limit` (optional, default: `20`) — Maximum clusters to return
- `status` (optional) — Filter by status: `open`, `dismissed`, `resolved`
- `severity` (optional) — Filter by severity: `low`, `medium`, `high`, `critical`

**Response:**
```json
[
  {
    "id": "uuid",
    "label": "Production Deployment Process",
    "description": "Multiple questions about deploying services to production went unanswered.",
    "query_count": 47,
    "sample_queries": [
      "How do I deploy to prod?",
      "What's the canary process?",
      "Where are the deployment configs?"
    ],
    "avg_confidence": 0.28,
    "severity": "critical",
    "status": "open",
    "unique_users": 12,
    "negative_ratio": 0.68,
    "trend": "recurring",
    "assignee_id": null,
    "assigned_at": null,
    "created_at": "2025-02-15T10:00:00Z",
    "updated_at": "2025-02-20T14:30:00Z"
  }
]
```

New fields vs. earlier versions:
- `unique_users` — distinct users who hit this gap
- `negative_ratio` — fraction of queries on this topic with negative feedback
- `trend` — `"new"` (appeared in last 7 days) or `"recurring"` (open > 7 days)
- `assignee_id` — UUID of the user assigned to resolve this gap, or `null`
- `assigned_at` — ISO timestamp when the gap was assigned, or `null`

---

### Trigger Gap Analysis

```
POST /api/v1/autopilot/analyze
```

Runs gap analysis immediately (normally runs on the `AUTOPILOT_GAP_ANALYSIS_INTERVAL_HOURS` schedule). Returns the number of new clusters created.

**Response:**
```json
{
  "new_clusters": 5
}
```

---

### Dismiss a Gap

```
POST /api/v1/autopilot/gaps/{cluster_id}/dismiss
```

Marks a gap cluster as dismissed (not worth addressing). Requires admin or editor role.

---

### Assign a Gap

```
POST /api/v1/autopilot/gaps/{cluster_id}/assign
```

Assigns a gap cluster to a user for resolution.

**Request Body:**
```json
{
  "user_id": "uuid-of-user-to-assign"
}
```

**Response:** `200 OK` with the updated gap cluster object.

---

### Gap Related Documents

```
GET /api/v1/autopilot/gaps/{cluster_id}/related-docs
```

Returns documents semantically related to this gap cluster — these are the docs that users were trying to get answers from when the gap was detected. Useful for identifying which authors to notify or which content needs updating.

**Response:**
```json
[
  {
    "source_id": "doc-123",
    "title": "Production Deployment Guide",
    "source_url": "https://confluence.example.com/...",
    "space": "PLATFORM",
    "freshness_score": 23.5,
    "author": "bhanu@example.com"
  }
]
```

---

### List Drafts

```
GET /api/v1/autopilot/drafts?status=pending_review&limit=20
```

**Query Parameters:**
- `status` (optional) — Filter by status: `pending_review`, `approved`, `published`, `rejected`
- `limit` (optional, default: `20`) — Maximum drafts to return

**Response:**
```json
[
  {
    "id": "uuid",
    "cluster_id": "uuid",
    "title": "Production Deployment Runbook",
    "content": "# Production Deployment\n\n## Prerequisites\n...",
    "content_type": "runbook",
    "source_queries": ["How do I deploy to prod?", "..."],
    "source_doc_ids": ["doc-uuid-1", "doc-uuid-2"],
    "quality_score": 0.87,
    "status": "pending_review",
    "created_at": "2025-02-20T15:00:00Z"
  }
]
```

---

### Get a Draft

```
GET /api/v1/autopilot/drafts/{draft_id}
```

Returns full draft content for review.

---

### Generate Draft for a Gap

```
POST /api/v1/autopilot/generate/{cluster_id}
```

Generates a draft document for the specified gap cluster. Uses existing docs as context. Also DMs Slack authors of related docs if `SLACK_BOT_TOKEN` is configured.

**Response:**
```json
{
  "draft_id": "uuid",
  "title": "Production Deployment Runbook",
  "content_type": "runbook",
  "quality_score": 0.87
}
```

---

### Update Draft Status

```
POST /api/v1/autopilot/drafts/{draft_id}/status
```

**Request Body:**
```json
{
  "status": "approved"
}
```

Valid statuses: `approved`, `published`, `rejected`

---

### Publish Draft

```
POST /api/v1/autopilot/drafts/{draft_id}/publish?target=github
```

Publishes an approved draft to the configured target system. Creates a page (Confluence), PR (GitHub), or MR (GitLab) with the draft content.

**Query Parameters:**
- `target` (optional) — Override the publish target: `confluence`, `github`, `gitlab`. If omitted, uses per-space routing (DB) → default config target.

**Response:**
```json
{
  "draft_id": "uuid",
  "status": "published",
  "target": "github",
  "url": "https://github.com/acme/docs/pull/42"
}
```

**Auth:** Editor role or above.

---

## Publish Targets

Manage per-space publish target routing. Admin-only endpoints.

### List Publish Targets

```
GET /api/v1/publish-targets
```

**Response:**
```json
[
  {
    "id": "uuid",
    "space": "PLATFORM",
    "target_type": "github",
    "config": {"repo": "acme/platform-docs", "token_env": "GITHUB_PUBLISH_TOKEN"},
    "priority": 10,
    "created_at": "2026-03-25T00:00:00Z",
    "updated_at": "2026-03-25T00:00:00Z"
  }
]
```

---

### Create Publish Target

```
POST /api/v1/publish-targets
```

**Request Body:**
```json
{
  "space": "PLATFORM",
  "target_type": "github",
  "config": {
    "repo": "acme/platform-docs",
    "token_env": "GITHUB_PUBLISH_TOKEN",
    "branch": "main",
    "docs_path": "docs"
  },
  "priority": 10
}
```

- `space` — Space key to route (omit or `null` for default/fallback target)
- `target_type` — `confluence`, `github`, or `gitlab`
- `config` — JSONB target configuration. **Must use `token_env` (env var name) instead of raw secrets.** Fields containing `token`, `api_token`, or `secret` are rejected.
- `priority` — Higher priority targets are chosen first (default: 0)

**Response:** `201 Created` with the created target.

**Errors:**
- `400` — Invalid target type or raw secrets in config
- `409` — A target for this space + type already exists

---

### Update Publish Target

```
PUT /api/v1/publish-targets/{id}
```

Partial update — only provided fields are changed.

**Request Body:**
```json
{
  "config": {"repo": "acme/new-docs-repo", "token_env": "GITHUB_PUBLISH_TOKEN"},
  "priority": 20
}
```

**Response:** `200 OK` with the updated target.

---

### Delete Publish Target

```
DELETE /api/v1/publish-targets/{id}
```

**Response:** `204 No Content`

---

### Weekly Digest Preview

```
GET /api/v1/autopilot/digest
```

Returns the current weekly digest data (without sending it to Slack).

**Response:**
```json
{
  "period_start": "2025-02-13T00:00:00Z",
  "period_end": "2025-02-20T00:00:00Z",
  "total_queries": 152,
  "unanswered_queries": 18,
  "top_gaps": [
    {
      "id": "uuid",
      "label": "Production Deployment Process",
      "query_count": 47,
      "severity": "critical",
      "trend": "recurring"
    }
  ],
  "new_drafts": [...],
  "stale_doc_count": 7,
  "top_docs_by_queries": [
    {
      "title": "API Rate Limits",
      "source_url": "https://...",
      "space": "PLATFORM",
      "retrieval_count": 94,
      "author": "bhanu@example.com"
    }
  ]
}
```

`top_docs_by_queries` — the 10 documents most frequently retrieved during the period, with author attribution. Gives doc owners insight into which content is being searched most heavily.

---

## Knowledge Health

### Health Report

```
GET /api/v1/health/report
```

Full knowledge base health overview.

**Response:**
```json
{
  "total_documents": 342,
  "overall_health_score": 67.3,
  "freshness_distribution": {
    "fresh": 120,
    "review": 89,
    "stale": 72,
    "outdated": 41,
    "archive": 20
  },
  "top_stale_cited_docs": [
    {
      "title": "Deploy Guide",
      "freshness_score": 23.0,
      "citations_last_7d": 47,
      "contradiction_score": 45.0
    }
  ],
  "coverage_gaps": 15
}
```

---

## Admin Endpoints

All admin endpoints require an admin-role API key.

### List API Keys

```
GET /api/v1/admin/keys
```

---

### Create API Key

```
POST /api/v1/admin/keys
```

**Request Body:**
```json
{
  "name": "Platform Team Key",
  "role": "editor",
  "allowed_spaces": ["PLATFORM", "SRE"]
}
```

`role`: `viewer`, `editor`, `analyst`, `admin`

- `viewer` — ask questions, browse answers, give feedback, and access all intelligence dashboards (Documentation Analytics, Predictive Gaps, Autonomous Document Maintenance, Knowledge Stream)
- `editor` — everything viewer can + manage spaces and captures
- `analyst` — everything editor can; reserved for future role-based scoping, currently equivalent to `editor`
- `admin` — full access including user management, RBAC config, and ingest triggers

`allowed_spaces`: hard-filters all queries and ingestion to the listed spaces. Empty array = no restriction.

---

### Revoke API Key

```
DELETE /api/v1/admin/keys/{key_id}
```

---

### Onboarding Mode

```
GET /api/v1/onboarding?role=platform-engineer&days=7
```

Returns an AI-curated reading list for a new team member.

**Query Parameters:**
- `role` — Job role or persona (e.g. `platform-engineer`, `sre`, `backend-developer`)
- `days` (optional, default: `7`) — Onboarding period in days

**Response:**
```json
{
  "role": "platform-engineer",
  "reading_list": [
    {
      "title": "Platform Onboarding Guide",
      "source_url": "https://...",
      "freshness_score": 85.0,
      "reason": "Direct onboarding guide covering role-specific processes and expectations."
    }
  ]
}
```

---

## Admin — MCP Manifests

Admin endpoints for inspecting and operating the MCP tool platform — viewing
merged tool catalogs, forcing discovery probes, and managing OAuth probe-user
designations for dynamic manifests.

All endpoints require the `admin` role.

### GET /api/v1/admin/mcp/manifests/{id}

Get full manifest detail including the merged tool catalog and discovery status.

**Response (200 OK):**
```json
{
  "id": "github",
  "active_version": 5,
  "display_name": "GitHub",
  "auth": { "...": "..." },
  "secrets": [ "..." ],

  "tools": [
    {
      "name": "github.issue_read",
      "description": "...",
      "tool_source": "discovered",
      "read_only": true,
      "args_schema": { "...": "..." },
      "output_size_cap_bytes": 16384,
      "latency_budget_ms": 7000,
      "upstream_name": "issue_read"
    }
  ],

  "discovery": {
    "mode": "dynamic",
    "refresh_seconds": 3600,
    "status": "ok",
    "last_attempt":  "2026-05-15T20:42:11Z",
    "last_success":  "2026-05-15T20:42:11Z",
    "last_error":    null,
    "next_scheduled": null,
    "collisions":    []
  },

  "probe_user": {
    "user_id": "uuid",
    "designated_at": "ISO-8601",
    "designated_by": "uuid",
    "last_probed_at": "ISO-8601|null"
  }
}
```

**Field semantics:**

- `tools[].tool_source` — one of `static` | `discovered` | `static_override`.
  Tools are merged from both sources; collisions surface in
  `discovery.collisions`.
- `discovery.mode` — `static` | `dynamic` | `unknown` (the last when the
  manifest isn't in the registry, e.g. after a discovery-disabled rollback).
- `discovery.status` — `not_applicable` | `pending` | `ok` | `failed` |
  `requires_probe_user` | `degraded_collisions`.
- `discovery.last_success` — populated for `ok` and `degraded_collisions`
  (degraded means the probe succeeded with name collisions; it is not a probe
  failure).
- `discovery.collisions[]` — names that appear in both static and discovered
  catalogs when neither side has `override_discovered: true`.
- `probe_user` — omitted entirely when no designation exists.

For static manifests `discovery.mode == "static"` and all other `discovery`
fields are `null`. Every tool has `tool_source: "static"`.

### POST /api/v1/admin/mcp/manifests/{id}/discover

Force an immediate probe of a dynamic MCP manifest. Synchronously runs the
probe and returns the new catalog, or surfaces the failure inline so the admin
UI can render it.

**Path params:**
- `{id}` — the manifest_id.

**Responses:**
- `200 OK` — `{ "outcome": "ok", "count": N, "tools": [...], "probed_at": "ISO-8601" }`
- `200 OK` — `{ "outcome": "failed", "error": { "kind": "...", "detail": "..." }, "probed_at": "ISO-8601" }`
- `404 Not Found` — manifest_id not in registry
- `409 Conflict` — manifest is static, OR a probe is already in flight for this manifest
- `503 Service Unavailable` — discovery worker not configured
- `504 Gateway Timeout` — probe did not complete within 7s
- `500 Internal Server Error` — worker dropped the reply channel (bug indicator)

`error.kind` is a stable snake_case token: `timeout` | `auth` | `http` |
`parse` | `transport` | `catalog_too_large` | `not_dynamic` | `not_found` |
`requires_probe_user` | `already_in_flight`. The UI can branch on this; `detail`
is human-readable.

**Audit:** writes `mcp.manifest.discover` to `audit_log` with `{outcome, count}`
on success or `{outcome, error_kind, error_detail}` on failure.

### GET /api/v1/admin/mcp/manifests/{id}/probe-user

Read the current OAuth probe-user designation. Returns the user designated to
provide OAuth credentials for periodic discovery probes on this manifest.

**Responses:**
- `200 OK` —
  ```json
  {
    "manifest_id": "github",
    "user_id": "uuid",
    "designated_at": "ISO-8601",
    "designated_by": "uuid",
    "last_probed_at": "ISO-8601|null"
  }
  ```
- `404 Not Found` — no probe user designated for this manifest

Unlike the other probe-user endpoints, this read succeeds even when MCP
discovery is disabled, so admins can audit historical designations after a
rollback.

### PUT /api/v1/admin/mcp/manifests/{id}/probe-user

Designate a user as the OAuth probe credential source. Validates that the user
has a non-revoked, non-expired OAuth token for this manifest before recording
the designation. The discovery worker will use this user's token for periodic
`tools/list` probes.

**Request:** `{ "user_id": "uuid" }`

**Responses:**
- `204 No Content` — designation recorded
- `409 Conflict` — user has no valid OAuth token for this manifest (the user
  must connect first via the standard OAuth flow)
- `503 Service Unavailable` — discovery worker not configured

**Audit:** writes `mcp.manifest.probe_user.set` with `{user_id}`.

### DELETE /api/v1/admin/mcp/manifests/{id}/probe-user

Remove the OAuth probe-user designation. After unset, the manifest's discovery
status flips to `requires_probe_user` on the next probe tick — no probes will
run until a new user is designated.

**Responses:**
- `204 No Content` — designation removed (or was already absent)
- `503 Service Unavailable` — discovery worker not configured

**Audit:** writes `mcp.manifest.probe_user.unset` with `{prior_user_id}` (the
user being un-designated, captured pre-delete for audit completeness).

### GET /api/v1/admin/principals

Search the principals table by case-insensitive prefix match on display name
or external id. Powers the admin UI's principal typeahead (e.g. when scoping a
tool to an SSO group or user).

**Query params:**
- `q` — prefix to match against `display` and `external_id`.
- `limit` — max results, clamped to `1..=100` (default `25`).

**Response (200 OK):**
```json
{
  "principals": [
    {
      "id": 3,
      "kind": "sso_group",
      "source": "sso",
      "external_id": "engineering",
      "display": "Engineering"
    }
  ]
}
```

### GET /api/v1/admin/mcp/manifests/{id}/usage

Aggregate the MCP audit log for a manifest over a rolling window. Powers the
admin UI Usage dashboard.

**Query params:**
- `days` — window size, clamped to `1..=90` (default `7`).

**Response (200 OK):**
```json
{
  "series":       [{ "day": "2026-05-19", "outcome": "ok", "count": 12 }],
  "top_users":    [{ "user_id": "uuid", "display": "Alice", "count": 9 }],
  "top_failures": [{ "tool_name": "jira.search", "args": {}, "error_class": "timeout", "count": 3 }]
}
```

**Field semantics:**
- `series[]` — daily invocation counts grouped by `outcome`.
- `top_users[]` — top 10 attributable users by invocation count.
- `top_failures[]` — top 5 failing `(tool_name, error_class)` groups, each with
  a redacted-args sample.

### POST /api/v1/admin/mcp/manifests/{id}/disable

Reversibly disable a manifest. Sets all of the manifest's enablements to
disabled, removing it from tool dispatch while preserving its scope
configuration — so re-enabling is a single step. Idempotent.

**Responses:**
- `200 OK` — manifest disabled (or was already disabled).

**Audit:** writes a manifest-disable entry to `audit_log`.

### DELETE /api/v1/admin/mcp/manifests/{id}

Irreversibly uninstall a manifest in a single transaction. Clears the
active-version pointer, then removes the manifest's OAuth tokens, probe users,
secrets, enablements, and all installed versions. Per-user OAuth tokens are
deleted — users must re-authorize on reinstall.

**Responses:**
- `200 OK` — manifest uninstalled.
- `404 Not Found` — manifest not installed.

**Audit:** writes a manifest-uninstall entry to `audit_log`.

---

## Admin — MCP Registry & Install

Admin endpoints for browsing the signed remote MCP registry, fetching a
single manifest from it, and installing a manifest in a single transactional
call. All endpoints require the `admin` role.

The 3 registry endpoints (`GET /registry`, `GET /registry/{id}/manifest`,
`POST /install-from-registry`) require `MCP_REGISTRY_PUBKEY` to be set at
boot. When unset the server boots normally and these endpoints return
`503 Service Unavailable`; admins can still install manifests via the
existing paste/URL flow.

### GET /api/v1/admin/mcp/registry

List the cached signed registry index.

The server fetches the index from `MCP_REGISTRY_URL`, verifies its Ed25519
signature against `MCP_REGISTRY_PUBKEY`, and caches it on disk at
`MCP_REGISTRY_CACHE_PATH` so subsequent calls survive transient network
outages (Tier 2 fallback).

**Response (200 OK):**
```json
{
  "schema_version": 1,
  "generated_at": "2026-05-15T20:42:11Z",
  "entries": [
    {
      "id": "github",
      "version": "1.4.2",
      "display_name": "GitHub",
      "summary": "Read-only GitHub MCP — issues, PRs, code search.",
      "manifest_url": "https://registry.docbrain-ai.com/v1/github/1.4.2.yaml",
      "manifest_sha256": "abc123…",
      "signed": true
    }
  ]
}
```

**Responses:**
- `200 OK` — index returned (from network or disk cache).
- `502 Bad Gateway` — both network fetch and disk cache failed (cold start
  with no connectivity).
- `503 Service Unavailable` — `MCP_REGISTRY_PUBKEY` not configured.

### GET /api/v1/admin/mcp/registry/{id}/manifest

Fetch and verify a single manifest by registry id. Used by the install
wizard to preview the manifest before committing.

**Path params:**
- `{id}` — registry entry id (e.g. `github`).

**Response (200 OK):**
```json
{
  "yaml": "id: github\ndisplay_name: GitHub\n...",
  "entry": {
    "id": "github",
    "version": "1.4.2",
    "display_name": "GitHub",
    "manifest_url": "https://registry.docbrain-ai.com/v1/github/1.4.2.yaml",
    "manifest_sha256": "abc123…",
    "signed": true
  }
}
```

**Responses:**
- `200 OK` — manifest YAML + index entry returned.
- `404 Not Found` — id not present in the registry index.
- `502 Bad Gateway` — manifest fetch failed or sha256 / signature
  verification failed.
- `503 Service Unavailable` — `MCP_REGISTRY_PUBKEY` not configured.

### POST /api/v1/admin/mcp/install-from-registry

Atomic install of a registry manifest. Fetches and verifies the manifest,
validates its schema, then in a single transaction writes
`mcp_installed_manifests`, sets `mcp_active_manifest_version`, creates
`mcp_manifest_enablements` for the requested scope, and records an
`audit_log` row with action `admin_mcp_install_from_registry`.

**Request:**
```json
{
  "id": "github",
  "version": "1.4.2",
  "allow_unsigned": false,
  "scope": { "type": "everyone" }
}
```

Or with a group scope:
```json
{
  "id": "github",
  "version": "1.4.2",
  "allow_unsigned": false,
  "scope": { "type": "groups", "group_ids": ["uuid1", "uuid2"] }
}
```

- `allow_unsigned` — MUST be `false`. Unsigned installs are not supported
  by this endpoint; use the existing paste/URL install endpoint instead
  (which renders its own per-manifest opt-in audit row).
- `scope.type` — `everyone` or `groups`. When `groups`, `group_ids` is
  required.

**Response (200 OK):**
```json
{
  "manifest_id": "github",
  "version": "1.4.2",
  "enabled_count": 12,
  "already_installed": false
}
```

Idempotent: re-installing the same `(id, version)` pair returns `200` with
`already_installed: true` and does not duplicate audit rows.

**Responses:**
- `200 OK` — installed (or already present).
- `400 Bad Request` — `allow_unsigned: true`, malformed scope, or schema
  validation failure on the fetched manifest.
- `404 Not Found` — `id` not present in the registry index.
- `502 Bad Gateway` — manifest fetch / verification failed.
- `503 Service Unavailable` — `MCP_REGISTRY_PUBKEY` not configured.

**Audit:** writes `admin_mcp_install_from_registry` with `{id, version,
scope, enabled_count}` on success.

### GET /api/v1/admin/mcp/secrets/audit/{manifest_id}

Inspect the running pod's environment for the env-var keys declared in
the manifest's `service_account.secret_refs` and render the kubectl
command to patch any missing ones.

**Path params:**
- `{manifest_id}` — the active manifest to audit.

**Response (200 OK):**
```json
{
  "manifest_id": "github",
  "required": ["GITHUB_TOKEN", "GITHUB_APP_ID"],
  "missing": ["GITHUB_APP_ID"],
  "secret_name": "docbrain-secrets",
  "namespace": "docbrain",
  "patch_command": "kubectl create secret generic docbrain-secrets --from-literal=GITHUB_APP_ID=<PASTE_VALUE> --dry-run=client -o yaml | kubectl apply -f -"
}
```

When `DOCBRAIN_K8S_SECRET_NAME` or `DOCBRAIN_K8S_NAMESPACE` are unset, the
rendered command contains the placeholder strings `<set
DOCBRAIN_K8S_SECRET_NAME>` / `<set DOCBRAIN_K8S_NAMESPACE>` so the admin
sees exactly what to configure.

**Operability:** env vars inject once at container start. After applying
the rendered patch the admin MUST restart the pod (e.g. `kubectl rollout
restart deployment/docbrain-server`) for the audit endpoint to reflect
the new values.

**Responses:**
- `200 OK` — audit result returned.
- `404 Not Found` — `manifest_id` unknown.

### POST /api/v1/admin/mcp/secrets/oauth

Reserved for v2 — programmatic OAuth-client-secret install via the
Kubernetes API.

**Response (501 Not Implemented):**
```json
{
  "error": "not_implemented",
  "hint": "Use the kubectl command rendered by GET /api/v1/admin/mcp/secrets/audit/{manifest_id} until v2 wires the Kubernetes client.",
  "kind": "v1_kubectl_only"
}
```

---

## Admin — Ownership Accuracy Audit

### POST /api/v1/admin/ownership/audit

Run an accuracy audit over the labeled ownership set. This evaluates how often the expertise scorer's confident attributions are wrong, and derives the confidence cutoff that meets a target error rate — the evidence operators use to decide whether to open the UI accuracy gate. Admin role required.

**Request body:**
```json
{
  "target_error": 0.05,
  "split_frac": 0.5
}
```
- `target_error` — the maximum confidently-wrong rate to target when deriving the confidence cutoff (e.g. `0.0`–`1.0`).
- `split_frac` — the calibration/test split fraction; clamped internally to `[0.1, 0.9]`.

**Response (200):** an accuracy summary containing the confident and abstain totals, the measured confidently-wrong rate, the area under the risk-coverage curve, the full risk-coverage curve, and the calibrate-then-test report.

---

## Knowledge Graph

### GET /api/v1/graph/entity/{name}

Disambiguate an entity by name and return its subgraph (neighbors and edges).

**Response:**
```json
{
  "ranked": [
    {
      "entity": { "id": "uuid", "name": "payments-service", "entity_type": "service" },
      "score": 0.95,
      "reason": "direct_connections=2, degree=15"
    }
  ],
  "subgraph": {
    "nodes": [
      { "id": "uuid", "name": "payments-service", "entity_type": "service" }
    ],
    "edges": [
      { "from_entity_id": "uuid1", "to_entity_id": "uuid2", "relation_type": "DEPENDS_ON" }
    ]
  }
}
```

### GET /api/v1/graph/dependencies/{entity_id}

Multi-hop dependency traversal from a given entity.

**Query params:** `depth` (default 2, max 5), `direction` (`downstream` | `upstream` | `both`), `relation_types` (comma-separated, optional)

**Response:**
```json
[
  {
    "entity": { "id": "uuid", "name": "auth-service", "entity_type": "service" },
    "depth": 1,
    "path": ["uuid1", "uuid2"]
  }
]
```

### GET /api/v1/graph/blast-radius/{entity_id}

Determine what is affected if an entity changes or goes down.

**Query params:** `depth` (default 3, max 5)

**Response:**
```json
{
  "entity": { "id": "uuid", "name": "payments-service", "entity_type": "service" },
  "affected": [{ "entity": { "id": "uuid", "name": "checkout-service", "entity_type": "service" }, "depth": 1, "path": [] }],
  "by_type": { "service": [{ "id": "uuid", "name": "checkout-service", "entity_type": "service" }] },
  "by_depth": { "1": [{ "id": "uuid", "name": "checkout-service", "entity_type": "service" }] }
}
```

### GET /api/v1/graph/path

Find the shortest path between two entities.

**Query params:** `from` (UUID, required), `to` (UUID, required), `depth` (max hops, default 5)

**Response:** Array of `GraphEdge` objects, or `null` if no path found within depth.

### GET /api/v1/graph/experts/{topic}

Route to domain experts via the entity-to-team-to-person chain.

**Response:**
```json
[
  {
    "person": { "id": "uuid", "name": "Alice Smith", "entity_type": "person" },
    "team": { "id": "uuid", "name": "platform-team", "entity_type": "team" },
    "confidence": 0.85,
    "route": []
  }
]
```

---

## Documentation Analytics

### GET /api/v1/analytics/velocity

Org-wide documentation analytics metrics over a configurable time window.

**Query params:** `days` (default 30)

**Response:**
```json
{
  "current_velocity": 2.5,
  "velocity_trend": "accelerating",
  "grade": "B",
  "knowledge_half_life_days": 45,
  "tribal_knowledge_pct": 0.15,
  "documentation_roi": {
    "queries_deflected": 340,
    "estimated_hours_saved": 85
  },
  "weekly_snapshots": [
    { "week": "2026-03-09", "velocity": 2.3, "docs_created": 4, "gaps_resolved": 2 }
  ],
  "per_team": [{ "team": "platform", "velocity": 3.2, "grade": "A" }]
}
```

### GET /api/v1/analytics/velocity/teams

Per-team velocity breakdown.

**Query params:** `days` (default 30)

### GET /api/v1/analytics/velocity/roi

Org-wide ROI summary: total queries deflected, hours saved, and cost saved in USD over the selected time window.

**Query params:** `days` (default 30)

**Response:**
```json
{
  "queries_deflected": 412,
  "hours_saved": 103.0,
  "cost_saved_usd": 7725.0,
  "days": 30
}
```

---

## Predictive Gap Detection

### POST /api/v1/predictive/code-change

Detect documentation that may be stale after a code change.

**Request:**
```json
{
  "changed_files": ["services/payments/handler.rs"],
  "pr_description": "Refactored payment flow"
}
```

**Response:** Array of `PredictedGap` objects with `doc_id`, `title`, `reason`, `confidence`, `trigger`.

### GET /api/v1/predictive/cascade

Detect cascade staleness — documents that reference recently-updated documents and may now be inconsistent.

### GET /api/v1/predictive/seasonal

Detect seasonal query patterns approaching their predicted peak for proactive refresh.

### GET /api/v1/predictive/onboarding

Detect onboarding gaps — common questions from new hires that are poorly covered or missing from documentation.

---

## Doc Maintenance

### GET /api/v1/maintenance/fixes

List auto-detected fix proposals (contradictions, broken links, version bumps).

**Query params:** `doc_id` (optional), `status` (`pending` | `approved` | `applied` | `rejected`), `limit` (default 50)

### POST /api/v1/maintenance/fixes/{id}/apply

Apply a fix proposal. **Requires authentication.**

**Response:** `200 OK`

### POST /api/v1/maintenance/fixes/{id}/reject

Reject a fix proposal. **Requires authentication.**

**Response:** `200 OK`

### GET /api/v1/maintenance/stats

Aggregate fix proposal statistics.

**Response:**
```json
{ "pending": 5, "approved": 2, "applied": 10, "rejected": 1 }
```

---

## Knowledge Stream

### GET /api/v1/stream/events

List recent stream events (incidents, decay alerts, expertise gaps, doc updates).

**Query params:** `since` (RFC3339), `type` (`incident_warning` | `decay_alert` | `expertise_gap` | `doc_updated`), `limit` (default 50)

### GET /api/v1/stream/events/user/{user_id}

Personalized event stream filtered by the user's active context.

### POST /api/v1/stream/context

Update user context (services and topics) for personalized stream delivery.

**Request:**
```json
{
  "services": ["payments", "auth"],
  "topics": ["latency", "deployment"]
}
```

### GET /api/v1/stream/stats

Event count statistics broken down by time window (24h, 7d, 30d) and event type.

## Event Bus

### GET /api/v1/events

Query the persistent event log. **Requires admin role.**

**Query params:**

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `type` | string | — | Filter by event type (e.g. `gap.detected`, `document.ingested`) |
| `since` | string | — | RFC3339 datetime or `YYYY-MM-DD` — only events after this time |
| `limit` | integer | `100` | Max results (1–1000) |
| `offset` | integer | `0` | Pagination offset |

**Response:**
```json
{
  "events": [
    {
      "id": "uuid",
      "event_type": "gap.detected",
      "payload": { "cluster_id": "uuid", "severity": "critical", "label": "...", "query_count": 15, "unique_users": 8 },
      "emitted_at": "2026-03-21T14:30:00Z",
      "processed_by": ["event_logger"]
    }
  ],
  "count": 1,
  "limit": 100,
  "offset": 0
}
```

**Event types:** `document.ingested`, `document.updated`, `document.deleted`, `freshness.changed`, `quality.scored`, `fragment.captured`, `fragment.indexed`, `fragment.promoted`, `gap.detected`, `gap.assigned`, `gap.resolved`, `draft.generated`, `draft.review_requested`, `draft.published`, `draft.rejected`, `query.answered`, `feedback.received`, `sla.breached`, `maintenance.fix_proposed`

### GET /api/v1/events/stream

SSE stream of real-time events. **Requires admin role.** Max 10 concurrent connections.

Each SSE message includes:
- `event:` — the event type (e.g. `gap.detected`)
- `id:` — unique event UUID (for `Last-Event-ID` reconnection)
- `data:` — JSON payload with the full `EventEnvelope` (id, event, emitted_at)

## Knowledge Fragments

### POST /api/v1/fragments

Create a new knowledge fragment. **Requires editor role.**

Fragments are routed by confidence: `>= auto_index_threshold` (default 0.7) → auto-indexed into search; `>= review_threshold` (default 0.4) → queued for review; below → auto-discarded.

**Request body:**
```json
{
  "fragment_type": "decision",
  "summary": "Switched from Redis pub/sub to PG LISTEN/NOTIFY",
  "content": "Redis cluster mode doesn't support pub/sub across shards...",
  "source_type": "pr_merge",
  "source_ref": "https://github.com/acme/platform/pull/1234",
  "source_id": "github:acme/platform#1234",
  "confidence": 0.85,
  "space": "PLATFORM",
  "related_doc_ids": ["550e8400-e29b-41d4-a716-446655440000"],
  "code_location": "src/events/publisher.rs:42"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `fragment_type` | string | yes | `decision`, `fact`, `caveat`, `procedure`, `context` |
| `summary` | string | yes | Short description |
| `content` | string | yes | Full content (max `FRAGMENT_MAX_CONTENT_LENGTH`) |
| `source_type` | string | yes | `pr_merge`, `commit`, `ide_annotation`, `conversation_distill`, `deploy`, `incident`, `manual`, `ci_analyze` |
| `source_ref` | string | no | URL or reference to the source |
| `source_id` | string | no | Dedup key (unique per source_type) |
| `confidence` | float | no | 0.0–1.0, default 0.5 |
| `space` | string | no | Space for routing/filtering |
| `related_doc_ids` | UUID[] | no | Related document IDs |
| `code_location` | string | no | File path and line (e.g. `src/foo.rs:42`) |

**Response:** `201 Created`
```json
{
  "id": "uuid",
  "status": "indexed",
  "routed_action": "auto_index"
}
```

### GET /api/v1/fragments

List fragments with optional filters.

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `status` | string | — | Filter by status: `pending`, `indexed`, `promoted`, `discarded`, `review_queued` |
| `space` | string | — | Filter by space |
| `source_type` | string | — | Filter by source type |
| `limit` | integer | `50` | Max results (1–1000) |
| `offset` | integer | `0` | Pagination offset |

### GET /api/v1/fragments/:id

Get a single fragment by ID.

### PATCH /api/v1/fragments/:id

Update a fragment. **Requires editor role.** Only provided fields are updated.

### DELETE /api/v1/fragments/:id

Delete a fragment. **Requires admin role.** Also removes from search index.

### GET /api/v1/fragments/review-queue

List fragments with `review_queued` status. **Requires analyst role.** Supports same filters as list.

### POST /api/v1/fragments/:id/approve

Approve a fragment — sets status to `indexed` and embeds/indexes into OpenSearch. **Requires analyst role.**

### POST /api/v1/fragments/:id/discard

Discard a fragment with optional reason. **Requires analyst role.**

**Request body:**
```json
{
  "reason": "Duplicate of existing documentation"
}
```

### GET /api/v1/fragments/stats

Fragment statistics — counts by status, source type, and space. Supports `?space=` filter.

---

### GET /api/v1/fragments/clusters

Discover semantic clusters of related fragments. Uses embedding similarity to group fragments by topic. **Requires analyst role.**

Returns clusters sorted by composability score (highest first). Clusters with `composable: true` meet the composition threshold (5+ fragments, diverse sources, 500+ words).

**Response:**
```json
[
  {
    "topic": "Redis Cluster Mode Limitations",
    "fragment_ids": ["uuid1", "uuid2", "uuid3", "uuid4", "uuid5"],
    "source_diversity": 3,
    "author_diversity": 4,
    "total_content_words": 1250,
    "composability_score": 0.85,
    "sample_summaries": [
      "Redis cluster mode doesn't support pub/sub across shards",
      "MULTI/EXEC transactions limited to single slot"
    ],
    "composable": true
  }
]
```

| Status | Meaning |
|--------|---------|
| 200 | Clusters returned |
| 403 | Insufficient role (requires analyst) |
| 409 | Clustering is disabled via configuration |

---

### POST /api/v1/fragments/clusters/compose

Compose a composable cluster into a documentation draft. Checks for existing doc coverage before composing — if a document already covers the topic (>70% similarity), composition is skipped. **Requires admin role.**

**Request Body:**
```json
{
  "cluster": {
    "topic": "Redis Cluster Mode Limitations",
    "fragment_ids": ["uuid1", "uuid2", "uuid3", "uuid4", "uuid5"],
    "source_diversity": 3,
    "author_diversity": 4,
    "total_content_words": 1250,
    "composability_score": 0.85,
    "sample_summaries": ["..."],
    "composable": true
  }
}
```

**Response:**
```json
{
  "draft_id": "uuid",
  "title": "Redis Cluster Mode Limitations",
  "fragment_count": 5
}
```

| Status | Meaning |
|--------|---------|
| 200 | Draft created successfully |
| 400 | Cluster not composable or invalid request |
| 403 | Insufficient role (requires admin) |
| 409 | Clustering disabled, or existing doc already covers this topic |

---

## Space Ownership & Governance

Explicit knowledge ownership — spaces get owners, maintainers, and contributors. Topics get stewards who are auto-assigned when matching gaps are detected. This is the accountability layer that ensures gaps get resolved and drafts get reviewed.

Governance is configured via API, not environment variables.

### GET /api/v1/governance/spaces

List all spaces with ownership summary (owner/maintainer/contributor counts). **Requires viewer role.**

**Response:**
```json
{
  "spaces": [
    { "space": "PLATFORM", "owner_count": 1, "maintainer_count": 2, "contributor_count": 5 },
    { "space": "INFRA", "owner_count": 0, "maintainer_count": 0, "contributor_count": 0 }
  ]
}
```

### GET /api/v1/governance/spaces/:space/owners

List owners, maintainers, and contributors for a specific space. **Requires viewer role.**

**Response:**
```json
{
  "owners": [
    {
      "id": "uuid",
      "space": "PLATFORM",
      "user_id": "uuid",
      "role": "owner",
      "notifications_enabled": true,
      "user_email": "alice@acme.com",
      "user_display_name": "Alice"
    }
  ]
}
```

### POST /api/v1/governance/spaces/:space/owners

Add a user as owner, maintainer, or contributor of a space. **Requires admin role.**

**Request body:**
```json
{
  "user_id": "uuid",
  "role": "owner",
  "notifications_enabled": true
}
```

Valid roles: `owner`, `maintainer`, `contributor`.

**Status codes:** `201` Created, `409` if user already assigned, `400` if user not found.

### DELETE /api/v1/governance/spaces/:space/owners/:user_id

Remove a user from a space's ownership. **Requires admin role.**

**Status codes:** `204` No Content, `404` if not found.

### PATCH /api/v1/governance/spaces/:space/owners/:user_id

Update a space owner's role or notification preference. **Requires admin role.**

**Request body:**
```json
{
  "role": "maintainer",
  "notifications_enabled": false
}
```

At least one field must be provided.

### GET /api/v1/governance/stewards

List all topic stewards with their regex patterns and auto-assign settings. **Requires viewer role.**

**Response:**
```json
{
  "stewards": [
    {
      "id": "uuid",
      "topic_pattern": "kubernetes|k8s|eks",
      "display_name": "Kubernetes Infrastructure",
      "user_id": "uuid",
      "auto_assign_gaps": true,
      "auto_assign_fragments": true,
      "user_email": "carol@acme.com",
      "user_display_name": "Carol"
    }
  ]
}
```

### POST /api/v1/governance/stewards

Create a topic steward. The `topic_pattern` is a regex matched against gap labels and fragment content for auto-assignment. **Requires admin role.**

**Request body:**
```json
{
  "topic_pattern": "kubernetes|k8s|eks",
  "display_name": "Kubernetes Infrastructure",
  "user_id": "uuid",
  "auto_assign_gaps": true,
  "auto_assign_fragments": true
}
```

Pattern validation: max 500 characters, must be valid regex. **Status codes:** `201` Created, `400` invalid pattern or user not found.

### GET /api/v1/governance/stewards/:id

Get a single topic steward by ID. **Requires viewer role.**

### DELETE /api/v1/governance/stewards/:id

Remove a topic steward. **Requires admin role.** Returns `204` or `404`.

### PATCH /api/v1/governance/stewards/:id

Update a topic steward's pattern, display name, or auto-assign settings. **Requires admin role.**

**Request body:**
```json
{
  "topic_pattern": "kubernetes|k8s|eks|aks",
  "display_name": "Kubernetes (all clouds)"
}
```

At least one field must be provided. New patterns are validated before saving.

### GET /api/v1/governance/my-spaces

List spaces the current user owns or maintains. Requires an API key with an associated `user_id`.

### GET /api/v1/governance/my-stewardships

List topics the current user stewards. Requires an API key with an associated `user_id`.

### GET /api/v1/governance/coverage

Ownership coverage report across all spaces. **Requires viewer role.**

**Response:**
```json
{
  "total_spaces": 12,
  "owned_spaces": 9,
  "coverage_pct": 75.0,
  "unowned_spaces": ["INFRA", "SECURITY", "ONBOARDING"]
}
```

Total spaces are derived from the `documents` table (distinct space values), not from governance tables — ensuring unowned spaces are visible.

### GET /api/v1/governance/dashboard

Aggregated governance overview combining ownership coverage, SLA compliance, quality scores, fragment stats, review queue, velocity, and top contributors. **Requires analyst role.**

Each section is independently fetched with a 5-second timeout. If a section fails, it returns `null` and the failure is recorded in `errors`. The endpoint always returns 200 with partial data.

**Response:**
```json
{
  "coverage": {
    "total_spaces": 12,
    "owned_spaces": 9,
    "coverage_pct": 75.0,
    "unowned_spaces": ["INFRA"]
  },
  "sla_breaches": {
    "total_open": 3,
    "total_acknowledged": 12,
    "by_type": [{ "sla_type": "gap_acknowledgment", "count": 2 }],
    "by_space": [{ "space": "INFRA", "count": 2 }]
  },
  "quality": {
    "overall_avg": 68.5,
    "by_space": [{ "space": "PAYMENTS", "avg_score": 72.3, "doc_count": 15 }],
    "worst_docs": [{ "document_id": "uuid", "title": "Old Guide", "composite_score": 23.4 }]
  },
  "fragments": {
    "total": 147,
    "by_status": [{ "status": "approved", "count": 89 }],
    "by_source_type": [{ "source_type": "pr_merge", "count": 68 }],
    "by_space": [{ "space": "PAYMENTS", "count": 45 }]
  },
  "review_queue": [
    {
      "draft_id": "uuid",
      "draft_title": "Deployment Runbook",
      "current_stage": "sme_review",
      "stage_display_name": "SME Review",
      "workflow_name": "Default",
      "space": "INFRA",
      "entered_stage_at": "2024-01-15T10:00:00Z"
    }
  ],
  "velocity": {
    "current_velocity": 1.2,
    "velocity_trend": "accelerating",
    "grade": "B",
    "weekly_snapshots": [
      { "week_start": "2024-01-08", "docs_created": 5, "docs_updated": 12, "gaps_opened": 3, "gaps_resolved": 4, "velocity": 1.1 }
    ]
  },
  "top_contributors": [
    { "author_id": "alice", "author_name": "Alice Smith", "fragment_count": 23, "approved_count": 18 }
  ],
  "errors": [],
  "generated_at": "2024-01-15T12:00:00Z"
}
```

Sections that fail to load return `null` with an entry in `errors`:
```json
{
  "errors": [
    { "section": "velocity", "reason": "timeout" }
  ]
}
```

---

## Governance SLAs

SLA policies define maximum acceptable times for gap acknowledgment, gap resolution, draft review, and document freshness. Policies can be set per-space or org-wide (default). A periodic background checker detects breaches and emits `SlaBreached` events.

### GET /api/v1/governance/slas

List all SLA policies. **Requires viewer role.**

**Response:**
```json
{
  "policies": [
    {
      "id": "uuid",
      "space": null,
      "gap_acknowledgment_hours": 48,
      "gap_resolution_days": 14,
      "draft_review_hours": 72,
      "freshness_review_days": 30,
      "created_at": "2025-01-01T00:00:00Z",
      "updated_at": "2025-01-01T00:00:00Z"
    }
  ]
}
```

The `space: null` row is the org-wide default (always present). Space-specific overrides appear with their space name.

### PUT /api/v1/governance/slas/default

Upsert the org-wide default SLA policy. **Requires admin role.**

**Request body** (all fields optional — omitted fields keep current values):
```json
{
  "gap_acknowledgment_hours": 48,
  "gap_resolution_days": 14,
  "draft_review_hours": 72,
  "freshness_review_days": 30
}
```

**Validation:** Hours must be 1–8760, days must be 1–365.

### PUT /api/v1/governance/slas/:space

Upsert a space-specific SLA policy override. **Requires admin role.** Same request body as above.

### DELETE /api/v1/governance/slas/:space

Delete a space-specific override (space falls back to org default). **Requires admin role.** Returns `204 No Content` on success, `404` if no override exists for the space.

### GET /api/v1/governance/breaches

List SLA breaches with optional filters. **Requires viewer role.**

**Query parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `space` | string | — | Filter by space |
| `sla_type` | string | — | Filter: `acknowledgment`, `resolution`, `review`, `freshness` |
| `open_only` | bool | `false` | Only show unacknowledged breaches |
| `limit` | int | `50` | Max results (up to 200) |
| `offset` | int | `0` | Pagination offset |

**Response:**
```json
{
  "breaches": [
    {
      "id": "uuid",
      "entity_type": "gap",
      "entity_id": "uuid",
      "sla_type": "acknowledgment",
      "space": "ENGINEERING",
      "owner_id": null,
      "hours_overdue": 12.5,
      "acknowledged_at": null,
      "acknowledged_by": null,
      "created_at": "2025-01-01T00:00:00Z",
      "updated_at": "2025-01-01T00:00:00Z"
    }
  ]
}
```

### GET /api/v1/governance/breaches/summary

Aggregate breach statistics for dashboards. **Requires viewer role.**

**Response:**
```json
{
  "total_open": 5,
  "total_acknowledged": 12,
  "by_type": [
    { "sla_type": "acknowledgment", "count": 3 },
    { "sla_type": "freshness", "count": 2 }
  ],
  "by_space": [
    { "space": "ENGINEERING", "count": 4 },
    { "space": "(org-wide)", "count": 1 }
  ]
}
```

### POST /api/v1/governance/breaches/:id/acknowledge

Mark a breach as acknowledged. **Requires editor role.** The acknowledging user is recorded. Returns `{ "acknowledged": true }` on success, `404` if the breach doesn't exist or was already acknowledged.

---

## Review Workflows

Configurable multi-stage review pipelines for autopilot drafts. Each space can have a workflow that defines approval stages (e.g., SME Review → Writer Review → Publish Approval). Reviewers approve, request changes, or reject drafts at each stage.

### GET /api/v1/governance/workflows

List all review workflows. **Requires viewer role.**

**Response:**
```json
{
  "workflows": [
    {
      "id": "uuid",
      "space": "ENGINEERING",
      "name": "Standard Review",
      "stages": [
        { "name": "sme_review", "required_role": "maintainer", "approvals_needed": 1 },
        { "name": "writer_review", "required_role": "contributor", "approvals_needed": 1 }
      ],
      "is_default": false,
      "created_at": "2026-03-22T10:00:00Z",
      "updated_at": "2026-03-22T10:00:00Z"
    }
  ]
}
```

### POST /api/v1/governance/workflows

Create a review workflow for a space. Each space can have at most one workflow. **Requires admin role.**

**Request body:**
```json
{
  "space": "ENGINEERING",
  "name": "Standard Review",
  "stages": [
    { "name": "sme_review", "required_role": "maintainer", "approvals_needed": 1 },
    { "name": "writer_review", "required_role": "contributor", "approvals_needed": 1 }
  ],
  "is_default": false
}
```

**Validation rules:**
- Maximum 10 stages per workflow
- Stage names must be unique within a workflow
- `required_role` must be one of: `owner`, `maintainer`, `contributor`
- `approvals_needed` must be between 1 and 20

### PATCH /api/v1/governance/workflows/:id

Update a workflow's name, stages, or default flag. **Requires admin role.**

### DELETE /api/v1/governance/workflows/:id

Delete a workflow. Drafts assigned to this workflow retain their current stage but lose the workflow reference (`ON DELETE SET NULL`). **Requires admin role.**

### POST /api/v1/drafts/:id/review

Submit a review action (approve, request changes, or reject) for a draft at its current stage. **Requires editor role.** The reviewer must also hold the stage's `required_role` in the draft's space governance.

**Request body:**
```json
{
  "action": "approve",
  "note": "Looks good, minor wording tweak suggested."
}
```

Valid actions: `approve`, `request_changes`, `reject`.

- **approve**: Counts toward the stage's `approvals_needed` threshold. When the threshold is met, the draft advances to the next stage (or becomes `reviewed` if it was the final stage).
- **request_changes**: Recorded but does not block approvals. The draft stays at the current stage.
- **reject**: Immediately sets the draft status to `rejected`.

Each reviewer can submit one action per stage. Submitting again updates the existing action (upsert).

**Response:** `200 OK` with `{"ok": true, "advanced": true}` if the draft advanced to the next stage.

### POST /api/v1/drafts/:id/skip-review

Admin bypass: skip all remaining review stages and move the draft directly to `reviewed` status. **Requires admin role.**

Use this when a draft doesn't need gatekeeping — e.g., trusted content sources, low-risk updates, or time-sensitive documentation.

**Response:** `200 OK` with `{"ok": true, "status": "reviewed"}`

Returns `404` if the draft is not found or not in a reviewable state (must be `status = 'draft'`).

### GET /api/v1/drafts/:id/reviews

List all review actions for a draft. **Requires viewer role.**

### GET /api/v1/drafts/:id/stage

Get the current review stage progress for a draft, including who has reviewed and how many approvals remain.

**Response:**
```json
{
  "draft_id": "uuid",
  "current_stage": "sme_review",
  "workflow_id": "uuid",
  "progress": {
    "stage_name": "sme_review",
    "approvals_needed": 2,
    "approvals_received": 1,
    "actions": [
      {
        "actor_id": "uuid",
        "action": "approve",
        "note": "LGTM",
        "created_at": "2026-03-22T11:00:00Z"
      }
    ]
  }
}
```

### POST /api/v1/drafts/:id/comments

Add a review comment to a draft. **Requires editor role.**

**Request body:**
```json
{
  "body": "Consider rephrasing the second paragraph for clarity.",
  "parent_id": null
}
```

Comment body is limited to 10,000 characters. `parent_id` enables threaded replies.

### GET /api/v1/drafts/:id/comments

List all comments on a draft, ordered by creation time. **Requires viewer role.**

### PATCH /api/v1/comments/:id

Update a comment's body. Only the comment author or an admin can edit. **Requires editor role.**

### POST /api/v1/comments/:id/resolve

Mark a comment as resolved. **Requires editor role.**

### GET /api/v1/reviews/my-queue

List drafts awaiting the current user's review, based on their space governance roles. Returns drafts where the user holds the required role for the current stage.

**Response:**
```json
{
  "items": [
    {
      "draft_id": "uuid",
      "title": "Getting Started with Kubernetes",
      "space": "ENGINEERING",
      "current_stage": "sme_review",
      "workflow_name": "Standard Review",
      "created_at": "2026-03-22T09:00:00Z"
    }
  ]
}
```

---

## Content Quality Scoring

Deterministic structural quality scores for documents and fragments. Each item receives a composite score (0-100) built from 7 sub-scores, with content-type-aware templates defining completeness expectations.

### List Quality Scores

```
GET /api/v1/quality/scores
```

Paginated list of quality scores with optional filters. **Requires viewer role.**

**Query Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `space` | string | — | Filter by space |
| `min_score` | float | — | Minimum composite score (0-100) |
| `max_score` | float | — | Maximum composite score (0-100) |
| `content_type` | string | — | Filter by content type |
| `limit` | integer | 50 | Max results (capped at 200) |
| `offset` | integer | 0 | Pagination offset |

**Response:**
```json
{
  "scores": [
    {
      "id": "uuid",
      "document_id": "uuid",
      "fragment_id": null,
      "heading_structure": 15.0,
      "section_completeness": 20.0,
      "code_presence": 10.0,
      "link_density": 7.0,
      "content_length": 8.0,
      "readability": 12.0,
      "metadata_quality": 10.0,
      "composite_score": 82.0,
      "scored_at": "2025-01-15T10:30:00Z"
    }
  ]
}
```

### Get Document Score

```
GET /api/v1/quality/scores/:doc_id
```

Quality score for a specific document. **Requires viewer role.**

**Response:**
```json
{
  "score": {
    "id": "uuid",
    "document_id": "uuid",
    "heading_structure": 15.0,
    "section_completeness": 20.0,
    "code_presence": 10.0,
    "link_density": 7.0,
    "content_length": 8.0,
    "readability": 12.0,
    "metadata_quality": 10.0,
    "composite_score": 82.0,
    "scored_at": "2025-01-15T10:30:00Z"
  },
  "status": "high"
}
```

Status values: `high` (80+), `acceptable` (60+), `needs_improvement` (40+), `poor` (<40).

### Trigger Rescore

```
POST /api/v1/quality/rescore
```

Triggers a rescore of all documents. Returns immediately — rescoring happens asynchronously during the next ingest cycle. **Requires admin role.**

**Response:**
```json
{
  "status": "accepted",
  "documents_to_score": 1234,
  "message": "Rescoring will happen during the next ingest cycle"
}
```

### Quality Report

```
GET /api/v1/quality/report
```

Aggregate quality report with per-space breakdown and worst-scoring documents. **Requires analyst role.**

**Response:**
```json
{
  "overall_avg": 72.5,
  "total_scored": 1234,
  "by_space": [
    {
      "space": "ENGINEERING",
      "avg_score": 78.3,
      "document_count": 450,
      "worst_docs": [
        {
          "document_id": "uuid",
          "title": "Legacy Migration Guide",
          "composite_score": 23.5
        }
      ]
    }
  ]
}
```

### List Content Type Templates

```
GET /api/v1/quality/templates
```

Returns the built-in content type templates that define section completeness expectations. **Requires viewer role.**

**Response:**
```json
{
  "templates": [
    {
      "content_type": "runbook",
      "required_sections": ["overview", "prerequisites", "steps", "rollback", "escalation"],
      "optional_sections": ["monitoring", "troubleshooting"],
      "min_word_count": 200,
      "max_word_count": 5000,
      "expect_code_blocks": true
    },
    {
      "content_type": "guide",
      "required_sections": ["introduction", "prerequisites", "steps"],
      "optional_sections": ["examples", "faq", "next steps"],
      "min_word_count": 300,
      "max_word_count": 10000,
      "expect_code_blocks": true
    }
  ]
}
```

Available content types: `runbook`, `guide`, `troubleshooting`, `faq`, `reference`.

### Sub-Score Breakdown

| Sub-Score | Range | What It Measures |
|-----------|-------|-----------------|
| `heading_structure` | 0-20 | Presence of headings, proper hierarchy (H1→H2→H3), no skipped levels |
| `section_completeness` | 0-25 | Required sections present per content type template |
| `code_presence` | 0-10 | Code blocks present when expected by content type |
| `link_density` | 0-10 | Internal/external links for cross-referencing |
| `content_length` | 0-10 | Word count within template-defined min/max range |
| `readability` | 0-15 | Sentence length variation, no wall-of-text paragraphs, manageable sentence lengths |
| `metadata_quality` | 0-10 | Author, source URL, and space metadata present |

### Semantic Quality (LLM-Assessed)

When semantic quality scoring is enabled, documents are also assessed by an LLM on four dimensions:

| Dimension | Range | Description |
|-----------|-------|-------------|
| `accuracy` | 0-25 | Claims grounded in cited sources |
| `completeness` | 0-25 | Covers all aspects a reader needs |
| `clarity` | 0-25 | Understandable without external help |
| `actionability` | 0-25 | Provides concrete steps, commands, and examples |

The semantic score (0-100) is stored in the `semantic_score` field with per-dimension details in `semantic_details`. The `composite_score` becomes a 50/50 blend of structural and semantic scores once both are available.

Semantic scoring runs as a background sweep (configurable interval, default 24h) and only evaluates documents with `structural_total >= 40` to avoid wasting LLM calls on obviously poor content. Newly generated drafts are scored immediately after creation.

---

## Style Rules Engine

Configurable linting rules for documentation consistency. Rules are scoped globally or per-space, with space-specific rules overriding global rules of the same type and name.

### List Style Rules

```
GET /api/v1/style-rules
```

**Requires viewer role.**

**Query Parameters:**

| Parameter | Type | Default | Description |
|-----------|------|---------|-------------|
| `space` | string | — | Filter by space |
| `rule_type` | string | — | Filter by type: `terminology`, `formatting`, `structure`, `custom_pattern` |
| `include_inactive` | boolean | `false` | Include inactive rules (admin only) |

**Response:**
```json
{
  "rules": [
    {
      "id": "uuid",
      "space": null,
      "rule_type": "terminology",
      "name": "avoid-simple",
      "description": "Avoid the word 'simple' — it dismisses reader difficulty",
      "config": { "term": "simple", "suggestion": "straightforward" },
      "severity": "warning",
      "is_active": true,
      "created_at": "2026-03-22T00:00:00Z",
      "updated_at": "2026-03-22T00:00:00Z"
    }
  ]
}
```

---

### Create Style Rule

```
POST /api/v1/style-rules
```

**Requires admin role.**

**Request Body:**
```json
{
  "space": null,
  "rule_type": "terminology",
  "name": "avoid-simple",
  "description": "Avoid the word 'simple'",
  "config": { "term": "simple", "suggestion": "straightforward" },
  "severity": "warning"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `space` | string | no | Space scope (`null` = global) |
| `rule_type` | string | yes | `terminology`, `formatting`, `structure`, `custom_pattern` |
| `name` | string | yes | Unique name (1-200 chars) |
| `description` | string | no | Human-readable description (max 2000 chars) |
| `config` | object | yes | Rule-type-specific configuration (see below) |
| `severity` | string | no | `error`, `warning`, `info` (default: `warning`) |

**Config by rule type:**

| Rule Type | Config Schema |
|-----------|---------------|
| `terminology` | `{ "term": "string", "suggestion": "string" }` |
| `formatting` | `{ "max_heading_depth": number }` or `{ "max_sentence_length": number }` |
| `structure` | `{ "require_intro": true }` |
| `custom_pattern` | `{ "pattern": "regex", "message": "string" }` |

**Response:** `201 Created`
```json
{
  "rule": { ... }
}
```

**Status codes:** `400` invalid input, `409` duplicate (space + type + name), `422` rule limit reached.

---

### Update Style Rule

```
PATCH /api/v1/style-rules/:id
```

**Requires admin role.** Only provided fields are updated.

**Request Body:**
```json
{
  "description": "Updated description",
  "config": { "term": "simple", "suggestion": "clear" },
  "severity": "error",
  "is_active": false
}
```

---

### Delete Style Rule

```
DELETE /api/v1/style-rules/:id
```

**Requires admin role.** Returns `204 No Content` or `404`.

---

### Import Rules from YAML

```
POST /api/v1/style-rules/import
```

**Requires admin role.** Upserts rules from a YAML string (max 100 rules per import). Existing rules with the same (space, type, name) are updated.

**Request Body:**
```json
{
  "yaml": "- rule_type: terminology\n  name: avoid-simple\n  config:\n    term: simple\n    suggestion: straightforward\n  severity: warning\n"
}
```

**Response:**
```json
{
  "imported": 3,
  "rules": [...]
}
```

---

### Export Rules to YAML

```
GET /api/v1/style-rules/export
```

**Requires admin role.** Returns all rules as a YAML document (`Content-Type: application/x-yaml`).

---

### Lint Content

```
POST /api/v1/quality/lint
```

**Requires analyst role.** Runs all active rules against the provided content.

**Request Body:**
```json
{
  "content": "This is a simple guide...",
  "space": "ENGINEERING"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | string | yes | Text to lint (max 500KB) |
| `space` | string | no | Space for rule scoping (global + space rules apply) |

**Response:**
```json
{
  "violations": [
    {
      "rule_name": "avoid-simple",
      "rule_type": "terminology",
      "severity": "warning",
      "message": "Avoid 'simple' — consider 'straightforward' instead",
      "line": 1,
      "column": 11,
      "span": "simple"
    }
  ],
  "style_score": 95.0,
  "summary": {
    "errors": 0,
    "warnings": 1,
    "infos": 0,
    "total": 1
  },
  "truncated": false
}
```

Style score formula: `max(0, 100 - (errors × 15 + warnings × 5 + infos × 1))`, clamped to [0, 100].

### Limits

| Limit | Value |
|-------|-------|
| Max rules per space | 200 |
| Max total rules | 1000 |
| Max import batch | 100 |
| Max lint content | 500 KB |
| Max violations per lint | 100 |
| Max regex pattern length | 500 chars |

---

## Webhooks

Outbound webhook subscriptions for pushing DocBrain events to external systems. All endpoints require **admin** role.

DocBrain signs every delivery with HMAC-SHA256 (`X-DocBrain-Signature` header), retries failed deliveries with exponential backoff, and automatically disables subscriptions after repeated failures (circuit breaker).

### List Webhook Subscriptions

```
GET /api/v1/webhooks
```

**Response:**
```json
[
  {
    "id": "uuid",
    "name": "Slack Pipeline",
    "url": "https://hooks.example.com/docbrain",
    "events": ["document.ingested", "gap.detected"],
    "headers": { "X-Custom": "value" },
    "is_active": true,
    "created_by": "uuid",
    "failure_count": 0,
    "last_failure_at": null,
    "last_success_at": "2026-03-22T10:00:00Z",
    "disabled_reason": null,
    "created_at": "2026-03-20T08:00:00Z",
    "updated_at": "2026-03-22T10:00:00Z"
  }
]
```

> **Note:** The `secret` field is never returned in API responses.

---

### Create Webhook Subscription

```
POST /api/v1/webhooks
```

**Request Body:**
```json
{
  "name": "Slack Pipeline",
  "url": "https://hooks.example.com/docbrain",
  "secret": "your-secret-at-least-16-chars",
  "events": ["document.ingested", "gap.detected"],
  "headers": { "X-Custom": "value" }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Human-readable subscription name |
| `url` | string | yes | HTTPS endpoint to deliver events to. Must not be a private/internal IP unless `ALLOW_INTERNAL_WEBHOOKS=true`. |
| `secret` | string | yes | HMAC signing secret (minimum 16 characters) |
| `events` | string[] | yes | Event types to subscribe to (see list below) |
| `headers` | object | no | Extra HTTP headers to include in deliveries |

**Response:** `201 Created` with the `WebhookSubscription` object.

**Validation:**
- `secret` must be at least 16 characters
- `events` must contain only valid event types
- `url` must not resolve to a private/internal IP address (unless `ALLOW_INTERNAL_WEBHOOKS=true`)

---

### Update Webhook Subscription

```
PATCH /api/v1/webhooks/:id
```

**Request Body:** (all fields optional)
```json
{
  "name": "Updated Name",
  "url": "https://new-endpoint.example.com/hook",
  "secret": "new-secret-at-least-16-chars",
  "events": ["document.ingested"],
  "headers": { "X-Custom": "new-value" },
  "is_active": true
}
```

**Response:** `200 OK` with the updated `WebhookSubscription` object.

---

### Delete Webhook Subscription

```
DELETE /api/v1/webhooks/:id
```

**Response:**
```json
{
  "status": "deleted"
}
```

---

### Send Test Event

```
POST /api/v1/webhooks/:id/test
```

Sends a `webhook.test` event to the subscription's URL. Useful for verifying connectivity and signature validation.

**Response:**
```json
{
  "success": true,
  "delivery_id": "uuid",
  "response_status": 200,
  "response_body": "OK"
}
```

---

### List Delivery Log

```
GET /api/v1/webhooks/:id/deliveries
```

Returns recent delivery attempts for a subscription.

**Query Parameters:**

| Param | Type | Default | Description |
|-------|------|---------|-------------|
| `limit` | integer | `50` | Max results |

**Response:** Array of `WebhookDelivery` objects with delivery ID, event type, HTTP status, response body, attempt number, and timestamps.

---

### Reset Circuit Breaker

```
POST /api/v1/webhooks/:id/reset
```

Resets the failure counter and re-enables a subscription that was auto-disabled by the circuit breaker.

**Response:**
```json
{
  "status": "reset",
  "is_active": true
}
```

---

### Event Types

All event types from the internal event bus are available for webhook subscriptions:

| Event Type | Description |
|------------|-------------|
| `document.ingested` | New document indexed |
| `document.updated` | Existing document re-indexed |
| `document.deleted` | Document removed |
| `freshness.changed` | Document freshness status changed |
| `quality.scored` | Quality score computed or updated |
| `fragment.captured` | Knowledge fragment captured |
| `fragment.indexed` | Fragment auto-indexed into search |
| `fragment.promoted` | Fragment promoted from review queue |
| `gap.detected` | New gap cluster detected |
| `gap.assigned` | Gap assigned to a user |
| `gap.resolved` | Gap marked as resolved |
| `draft.generated` | AI draft generated for a gap |
| `draft.review_requested` | Draft sent for review |
| `draft.published` | Draft published to target system |
| `draft.rejected` | Draft rejected during review |
| `query.answered` | User query answered |
| `feedback.received` | User feedback submitted |
| `sla.breached` | Documentation SLA breached |
| `maintenance.fix_proposed` | Automated fix proposal generated |

### Delivery Headers

Every webhook delivery includes these headers:

| Header | Description |
|--------|-------------|
| `Content-Type` | `application/json` |
| `X-DocBrain-Signature` | HMAC-SHA256 signature: `sha256=<hex-digest>`. Compute `HMAC-SHA256(secret, raw_body)` and compare to verify authenticity. |
| `X-DocBrain-Event` | Event type (e.g. `document.ingested`) |
| `X-DocBrain-Delivery` | Unique delivery UUID for idempotency |

### Retry Policy

Failed deliveries (non-2xx response or timeout) are retried with exponential backoff:

| Attempt | Delay |
|---------|-------|
| 1 | Immediate |
| 2 | 60 seconds |
| 3 | 300 seconds (5 minutes) |
| 4 | 3600 seconds (1 hour) |

After all retry attempts are exhausted, the failure counter is incremented. When the failure counter reaches `WEBHOOK_CIRCUIT_BREAKER_THRESHOLD` (default: 10) consecutive failures, the subscription is automatically disabled with `disabled_reason: "circuit_breaker"`. Use `POST /api/v1/webhooks/:id/reset` to re-enable.

---

## External Connectors

External connectors are stateless HTTP servers that implement a simple REST contract. DocBrain calls them on a cron schedule to ingest documents. All management endpoints require **admin** role. Status endpoint requires **viewer+** role.

### List Connectors

```
GET /api/v1/connectors
```

**Response:**
```json
[
  {
    "id": "uuid",
    "name": "internal-wiki",
    "display_name": "Internal Wiki",
    "base_url": "https://wiki-connector.example.com",
    "source_type": "wiki",
    "schedule_cron": "0 */6 * * *",
    "space": "engineering",
    "is_active": true,
    "last_sync_at": "2026-03-24T06:00:00Z",
    "last_sync_docs": 42,
    "last_error": null,
    "consecutive_failures": 0,
    "created_at": "2026-03-20T08:00:00Z",
    "updated_at": "2026-03-24T06:00:00Z"
  }
]
```

> **Note:** The `auth_header` field is never returned in API responses.

---

### Create Connector

```
POST /api/v1/connectors
```

**Request Body:**
```json
{
  "name": "internal-wiki",
  "display_name": "Internal Wiki",
  "base_url": "https://wiki-connector.example.com",
  "auth_header": "Bearer your-token",
  "source_type": "wiki",
  "schedule_cron": "0 */6 * * *",
  "space": "engineering"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Unique machine name (1-100 chars) |
| `display_name` | string | yes | Human-readable name (1-100 chars) |
| `base_url` | string | yes | Connector HTTP endpoint (1-2048 chars, must start with `http://` or `https://`) |
| `auth_header` | string | no | Authorization header value (e.g. `Bearer token`) |
| `source_type` | string | yes | Document source type (1-20 chars, must be unique across connectors) |
| `schedule_cron` | string | no | Standard 5-field cron expression (default: `0 */6 * * *` = every 6 hours) |
| `space` | string | no | Target space for ingested documents |

**Response:** `201 Created` with the connector object.

**Validation:**
- `base_url` must not resolve to a private/internal IP address (unless `CONNECTOR_ALLOW_INTERNAL=true`)
- `schedule_cron` must be a valid cron expression
- `source_type` must be unique — returns `409 Conflict` if already registered

---

### Update Connector

```
PATCH /api/v1/connectors/:id
```

**Request Body:** (all fields optional)
```json
{
  "display_name": "Updated Wiki",
  "base_url": "https://new-url.example.com",
  "auth_header": "Bearer new-token",
  "source_type": "wiki",
  "schedule_cron": "0 */12 * * *",
  "space": "docs",
  "is_active": true
}
```

**Response:** `200 OK` with the updated connector object. Returns `404` if not found.

---

### Delete Connector

```
DELETE /api/v1/connectors/:id
```

**Response:** `200 OK` with `{"deleted": true}`. Returns `404` if not found.

---

### Trigger Manual Sync

```
POST /api/v1/connectors/:id/sync
```

Triggers an immediate sync for the connector, bypassing the cron schedule and circuit breaker. Returns an error if a sync is already in progress for this connector.

**Response:**
```json
{
  "docs_synced": 15
}
```

| Status | Meaning |
|--------|---------|
| 200 | Sync completed successfully |
| 404 | Connector not found |
| 409 | Sync already in progress |

---

### Connector Status

```
GET /api/v1/connectors/:id/status
```

Returns sync health information for a connector. Requires **viewer+** role.

**Response:**
```json
{
  "id": "uuid",
  "name": "internal-wiki",
  "is_active": true,
  "last_sync_at": "2026-03-24T06:00:00Z",
  "last_sync_docs": 42,
  "last_error": null,
  "consecutive_failures": 0
}
```

---

### Test Connector Health

```
POST /api/v1/connectors/:id/test
```

Runs a health check against the connector's `/health` endpoint without triggering a sync.

**Response:**
```json
{
  "status": "ok",
  "version": "1.0.0"
}
```

If the health check fails:
```json
{
  "status": "error",
  "error": "Connection refused"
}
```

---

### Connector Protocol

External connectors must implement three endpoints:

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/health` | GET | Returns `{"status": "ok"}` and optionally `{"version": "..."}` |
| `/documents/list` | POST | Lists available documents (paginated, supports incremental sync via `since`) |
| `/documents/fetch` | POST | Returns full document content for a list of source IDs |

**List Request:**
```json
{
  "since": "2026-03-20T00:00:00Z",
  "page": 1,
  "page_size": 50
}
```

**List Response:**
```json
{
  "documents": [
    { "source_id": "doc-123", "title": "Getting Started", "updated_at": "2026-03-24T10:00:00Z" }
  ],
  "has_more": false,
  "total": 1
}
```

**Fetch Request:**
```json
{
  "source_ids": ["doc-123", "doc-456"]
}
```

**Fetch Response:**
```json
{
  "documents": [
    {
      "source_id": "doc-123",
      "title": "Getting Started",
      "content": "# Getting Started\n\nWelcome to...",
      "content_type": "markdown",
      "url": "https://wiki.example.com/getting-started",
      "author": "Jane Doe",
      "updated_at": "2026-03-24T10:00:00Z",
      "metadata": {},
      "references": []
    }
  ]
}
```

---

## CI/CD Pipeline Capture

Automated knowledge extraction from merged PRs and deployments. Requires **editor** role.

### POST /api/v1/ci/analyze

Analyze a merged PR and extract knowledge fragments.

**Request body:**

```json
{
  "pr_number": 1234,
  "repo": "acme/platform",
  "pr_title": "Switch event delivery to PG LISTEN/NOTIFY",
  "pr_body": "Replaced Redis pub/sub with PostgreSQL...",
  "diff_stat": "+120 -45",
  "changed_files": "src/events/publisher.rs,src/events/subscriber.rs",
  "labels": "architecture,breaking-change",
  "author": "alice@acme.com"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `pr_number` | integer | yes | Pull request number |
| `repo` | string | yes | Repository in `owner/name` format |
| `pr_title` | string | yes | PR title (max 500 chars) |
| `pr_body` | string | no | PR description (max 50,000 chars) |
| `diff_stat` | string | no | Git diff stats |
| `changed_files` | string | no | Comma-separated list of changed files |
| `labels` | string | no | Comma-separated PR labels |
| `author` | string | no | PR author email or username |

**Response:**

```json
{
  "fragments_created": 2,
  "fragments": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "fragment_type": "decision",
      "summary": "Switched event delivery from Redis to PG LISTEN/NOTIFY",
      "confidence": 0.85,
      "routed_action": "auto_index"
    }
  ],
  "already_analyzed": false
}
```

| Status | Description |
|--------|-------------|
| 200 | Analysis complete (check `fragments_created` and `already_analyzed`) |
| 400 | Validation error (missing required field, field too long) |
| 403 | Insufficient role (requires editor+) |
| 503 | CI analysis is disabled (`CI_ANALYZE_ENABLED=false`) |

The endpoint is **idempotent**: re-analyzing the same PR returns `already_analyzed: true` with no new fragments.

### POST /api/v1/ci/deploy-capture

Capture deployment context as a knowledge fragment.

**Request body:**

```json
{
  "service": "payment-gateway",
  "version": "2.4.1",
  "environment": "production",
  "changelog": "abc1234 Fixed retry logic\ndef5678 Updated timeout config",
  "config_diff": "timeout: 30 -> 60"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `service` | string | yes | Service name (max 256 chars) |
| `version` | string | yes | Version being deployed (max 128 chars) |
| `environment` | string | yes | Target environment (max 128 chars) |
| `changelog` | string | no | Git changelog (max 50,000 chars) |
| `config_diff` | string | no | Configuration changes (max 50,000 chars) |

**Response:**

```json
{
  "fragment_id": "550e8400-e29b-41d4-a716-446655440000",
  "summary": "Deployed payment-gateway v2.4.1 to production with retry logic fix"
}
```

| Status | Description |
|--------|-------------|
| 200 | Capture complete |
| 400 | Validation error |
| 403 | Insufficient role |
| 503 | CI analysis is disabled |

### GitHub Action Setup

Add this workflow to your repository to automatically capture knowledge from merged PRs:

```yaml
name: DocBrain Knowledge Capture
on:
  pull_request:
    types: [closed]

jobs:
  capture:
    if: github.event.pull_request.merged == true
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 2
      - name: Capture knowledge from PR
        env:
          DOCBRAIN_API_URL: ${{ secrets.DOCBRAIN_API_URL }}
          DOCBRAIN_API_KEY: ${{ secrets.DOCBRAIN_API_KEY }}
        run: |
          curl -s -X POST "${DOCBRAIN_API_URL}/api/v1/ci/analyze" \
            -H "Authorization: Bearer ${DOCBRAIN_API_KEY}" \
            -H "Content-Type: application/json" \
            -d "$(jq -n \
              --argjson pr_number ${{ github.event.pull_request.number }} \
              --arg repo '${{ github.repository }}' \
              --arg pr_title '${{ github.event.pull_request.title }}' \
              '{pr_number: $pr_number, repo: $repo, pr_title: $pr_title}')"
```

Required repository secrets:
- `DOCBRAIN_API_URL` — Your DocBrain server URL
- `DOCBRAIN_API_KEY` — API key with editor+ role
