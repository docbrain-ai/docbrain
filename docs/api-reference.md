# API Reference

Base URL: `http://localhost:3000` (default)

Most endpoints require authentication via Bearer token or API key:

```
Authorization: Bearer db_sk_...
```

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
  "intent": "procedural"
}
```

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
```

**Query Parameters:**
- `space` (optional) — Filter by document space

**Response:**
```json
{
  "space": "DOCS",
  "summary": {
    "total_docs": 142,
    "fresh": 98,
    "review": 27,
    "stale": 12,
    "outdated": 5,
    "avg_score": 76.3
  },
  "documents": [
    {
      "document_id": "123",
      "title": "API Guide",
      "space": "DOCS",
      "source_url": "https://...",
      "total_score": 45.2,
      "status": "stale",
      "freshness_badge": "🟡 Review",
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
