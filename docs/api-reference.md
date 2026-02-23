# API Reference

Base URL: `http://localhost:3000` (default)

All endpoints except `/api/v1/config` require authentication via Bearer token.

```
Authorization: Bearer db_sk_...
```

## Endpoints

### Health Check

```
GET /api/v1/health
```

Returns `200 OK` with `{"status": "ok"}`. Requires authentication.

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
  "stream": true
}
```

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
      "score": 0.92
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

`feedback`: `1` (helpful) or `-1` (not helpful)

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
GET /api/v1/analytics?days=30
```

**Query Parameters:**
- `days` (optional, default: 30) — Reporting period

**Response:**
```json
{
  "period_days": 30,
  "total_queries": 1247,
  "unique_users": 45,
  "avg_feedback": 0.82,
  "queries_by_day": [
    {"date": "2025-01-15", "count": 42}
  ],
  "top_intents": [
    {"intent": "procedural", "count": 456}
  ],
  "doc_gaps": [
    {"query": "kubernetes networking", "count": 12}
  ]
}
```

---

### Server Configuration

```
GET /api/v1/config
```

No authentication required. Returns enabled features and server version.

```json
{
  "version": "0.1.0",
  "features": {
    "freshness": true,
    "analytics": true,
    "slack": false,
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

### List Gap Clusters

```
GET /api/v1/autopilot/gaps?limit=20
```

**Query Parameters:**
- `limit` (optional, default: 20) — Maximum clusters to return

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
    "created_at": "2025-02-15T10:00:00Z",
    "updated_at": "2025-02-20T14:30:00Z"
  }
]
```

---

### Trigger Gap Analysis

```
POST /api/v1/autopilot/analyze
```

Runs gap analysis immediately (normally runs on a daily schedule). Returns the number of new clusters created.

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

Marks a gap cluster as dismissed (not worth addressing).

---

### List Drafts

```
GET /api/v1/autopilot/drafts?status=pending_review&limit=20
```

**Query Parameters:**
- `status` (optional) — Filter by status: `pending_review`, `approved`, `published`, `rejected`
- `limit` (optional, default: 20) — Maximum drafts to return

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

Generates a draft document for the specified gap cluster. Uses existing docs as context.

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

Returns the current weekly digest data (without sending it).

**Response:**
```json
{
  "period_start": "2025-02-13T00:00:00Z",
  "period_end": "2025-02-20T00:00:00Z",
  "total_queries": 152,
  "unanswered_queries": 18,
  "top_gaps": [...],
  "new_drafts": [...],
  "stale_doc_count": 7
}
```
