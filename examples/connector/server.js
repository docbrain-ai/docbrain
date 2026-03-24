const express = require("express");
const app = express();
app.use(express.json());

const PORT = process.env.PORT || 4000;

// ─── In-memory wiki documents ───────────────────────────────────────────────
// In production this would query a database, Notion API, Google Docs, etc.
const documents = [
  {
    source_id: "wiki-001",
    title: "Team OKRs — Q1 2026",
    content: `# Team OKRs — Q1 2026

## Platform Team
- **O1:** Reduce deployment time by 40%
  - KR1: P95 deploy time < 5 minutes (current: 8 min)
  - KR2: Zero-downtime deploys for 100% of services
  - KR3: Self-service rollback available to all teams

- **O2:** Improve observability coverage
  - KR1: 100% of services have SLOs defined
  - KR2: Mean time to detect (MTTD) < 2 minutes
  - KR3: Distributed tracing coverage > 95%

## Payments Team
- **O1:** Launch multi-currency support
  - KR1: Support 12 currencies (current: 4)
  - KR2: Currency conversion accuracy > 99.9%
  - KR3: Settlement time < 3 business days for all currencies

## Search Team
- **O1:** Migrate to vector search
  - KR1: Implement hybrid search (BM25 + vector)
  - KR2: Search relevance score > 0.85 (current: 0.72)
  - KR3: P99 search latency < 200ms`,
    content_type: "markdown",
    url: "https://wiki.acme-platform.internal/okrs/q1-2026",
    author: "Sarah Chen",
    updated_at: "2026-01-15T10:00:00Z",
    metadata: { category: "planning", quarter: "Q1-2026" },
  },
  {
    source_id: "wiki-002",
    title: "Architecture Decision Record: Event-Driven Payments",
    content: `# ADR-042: Migrate Payments to Event-Driven Architecture

## Status
Accepted (2026-02-10)

## Context
The payments service currently uses synchronous HTTP calls for all downstream communication. This creates tight coupling, makes it difficult to add new consumers, and causes cascade failures when downstream services are slow.

## Decision
We will migrate the payments service to an event-driven architecture using Amazon EventBridge:

1. **Payment state changes** will be published as domain events
2. **Downstream consumers** (notifications, analytics, reconciliation) will subscribe to relevant events
3. **Synchronous calls** will be retained only for payment processor communication (Stripe/Adyen)

## Event Schema
\`\`\`json
{
  "source": "payments-service",
  "detail-type": "PaymentCompleted",
  "detail": {
    "payment_id": "pay_xxx",
    "amount": 4999,
    "currency": "usd",
    "customer_id": "cust_xxx",
    "timestamp": "2026-02-10T14:30:00Z"
  }
}
\`\`\`

## Consequences
- **Positive:** Loose coupling, easy to add consumers, better fault isolation
- **Negative:** Eventual consistency (consumers may see events with 100-500ms delay)
- **Risk:** Event ordering is not guaranteed across partitions — consumers must be idempotent

## Participants
- Marcus Rivera (decision maker)
- Priya Patel (reviewer)
- Alex Kim (security review)`,
    content_type: "markdown",
    url: "https://wiki.acme-platform.internal/adr/042",
    author: "Marcus Rivera",
    updated_at: "2026-02-10T09:00:00Z",
    metadata: { category: "adr", adr_number: "042" },
  },
  {
    source_id: "wiki-003",
    title: "Production Readiness Checklist",
    content: `# Production Readiness Checklist

Every service must pass this checklist before receiving production traffic. The Platform team reviews submissions within 2 business days.

## Required

### Reliability
- [ ] Health check endpoint (\`GET /health\`) returns 200 within 100ms
- [ ] Graceful shutdown handles in-flight requests (30s drain period)
- [ ] Circuit breakers configured for all external dependencies
- [ ] Retry logic with exponential backoff for transient failures
- [ ] Request timeouts set for all HTTP clients (default: 10s)

### Observability
- [ ] Structured JSON logging with trace IDs
- [ ] Prometheus metrics exposed at \`/metrics\`
- [ ] SLOs defined and alert rules created
- [ ] Grafana dashboard created with request rate, error rate, and latency
- [ ] Distributed tracing propagation configured

### Security
- [ ] No secrets in code or configuration files
- [ ] Vault integration for secret management
- [ ] TLS for all external communication
- [ ] Input validation on all API endpoints
- [ ] Rate limiting configured

### Operations
- [ ] Runbook written and linked in PagerDuty
- [ ] On-call rotation assigned
- [ ] Rollback procedure documented and tested
- [ ] Resource limits (CPU/memory) set in Kubernetes manifest
- [ ] Autoscaling configured with appropriate min/max

### Data
- [ ] Database migrations are backwards-compatible
- [ ] Backup and recovery procedure documented
- [ ] Data retention policy defined
- [ ] PII handling reviewed by security team

## How to Submit

1. Open a Production Readiness Review ticket in Jira (project: PLATFORM)
2. Attach the completed checklist
3. Tag the Platform team lead for review
4. Address any feedback within 5 business days

## Exemptions

Time-sensitive launches may request a temporary exemption from specific items. Exemptions must be approved by the VP Engineering and include a remediation timeline (maximum 30 days).`,
    content_type: "markdown",
    url: "https://wiki.acme-platform.internal/production-readiness",
    author: "Priya Patel",
    updated_at: "2026-03-01T14:00:00Z",
    metadata: { category: "standards" },
  },
  {
    source_id: "wiki-004",
    title: "Post-Mortem: 2026-03-05 Checkout Outage",
    content: `# Post-Mortem: Checkout Service Outage

**Date:** March 5, 2026
**Duration:** 47 minutes (14:23 - 15:10 UTC)
**Severity:** SEV-1
**Impact:** 100% of checkout requests failed, estimated revenue loss ~$85,000

## Summary
A misconfigured database migration caused the checkout service to create a full table lock on the \`orders\` table. All write operations queued behind the lock, causing connection pool exhaustion and cascading failures to the payments and inventory services.

## Timeline
- **14:20** — Engineer deploys migration adding a new index to the \`orders\` table
- **14:23** — PagerDuty alert: checkout error rate > 5%
- **14:25** — On-call engineer acknowledges, begins investigation
- **14:32** — Root cause identified: \`CREATE INDEX\` without \`CONCURRENTLY\` keyword
- **14:35** — Decision to kill the migration query
- **14:38** — \`pg_terminate_backend()\` kills the locking query
- **14:42** — Connection pools begin recovering
- **14:55** — Error rate returns to baseline
- **15:10** — All queued requests processed, incident resolved

## Root Cause
The migration script used \`CREATE INDEX\` instead of \`CREATE INDEX CONCURRENTLY\`. On a table with 50M+ rows, this acquired an exclusive lock that blocked all writes for the duration of index creation (estimated 15+ minutes).

## Action Items
1. **[P1]** Add CI check that rejects non-concurrent index creation on tables > 1M rows — @priya
2. **[P1]** Add migration review requirement for any DDL on production tables — @marcus
3. **[P2]** Implement migration dry-run step in staging that validates lock behavior — @platform-team
4. **[P3]** Document safe migration patterns in the engineering wiki — @sarah

## Lessons Learned
- Database migrations are the #1 cause of SEV-1 incidents (3 of last 5)
- We need automated guardrails, not just documentation
- The team's response time (12 minutes to root cause) was excellent`,
    content_type: "markdown",
    url: "https://wiki.acme-platform.internal/postmortems/2026-03-05-checkout",
    author: "Priya Patel",
    updated_at: "2026-03-07T16:00:00Z",
    metadata: { category: "postmortem", severity: "SEV-1" },
  },
  {
    source_id: "wiki-005",
    title: "API Versioning Strategy",
    content: `# API Versioning Strategy

## Approach
We use URL path versioning (\`/api/v1/...\`, \`/api/v2/...\`) for all public APIs. Internal service-to-service APIs use header versioning via \`Accept: application/vnd.acme.v2+json\`.

## Version Lifecycle

| Phase | Duration | Support Level |
|-------|----------|---------------|
| Current | Until next major version | Full support, new features |
| Deprecated | 12 months after successor | Security fixes only |
| Sunset | End of deprecation | Requests return 410 Gone |

## Breaking Change Policy

The following are considered breaking changes:
- Removing a field from a response
- Changing a field type
- Adding a required request field
- Changing error response format
- Removing an endpoint

**Non-breaking changes** (safe to add without version bump):
- Adding optional request fields
- Adding new response fields
- Adding new endpoints
- Adding new enum values (if clients handle unknown values)

## Current API Versions

| API | Current | Deprecated | Sunset |
|-----|---------|------------|--------|
| Orders | v2 | v1 (June 2026) | v1 (Dec 2026) |
| Payments | v1 | — | — |
| Users | v3 | v2 (March 2026) | v1 (already sunset) |
| Search | v1 | — | — |

## Migration Guide

When migrating from v1 to v2:
1. Review the changelog for breaking changes
2. Update your client SDK to the latest version
3. Test against the staging v2 endpoint
4. Switch production traffic with a feature flag
5. Monitor error rates for 48 hours
6. Remove v1 code paths after stabilization`,
    content_type: "markdown",
    url: "https://wiki.acme-platform.internal/api-versioning",
    author: "Marcus Rivera",
    updated_at: "2026-02-20T11:00:00Z",
    metadata: { category: "standards" },
  },
];

// ─── GET /health ────────────────────────────────────────────────────────────
app.get("/health", (_req, res) => {
  res.json({
    status: "ok",
    connector_name: "acme-wiki",
    version: "1.0.0",
    document_count: documents.length,
  });
});

// ─── POST /documents/list ───────────────────────────────────────────────────
app.post("/documents/list", (req, res) => {
  const { since, page = 1, page_size = 50 } = req.body;

  let filtered = documents;
  if (since) {
    const sinceDate = new Date(since);
    filtered = documents.filter((d) => new Date(d.updated_at) > sinceDate);
  }

  const start = (page - 1) * page_size;
  const paged = filtered.slice(start, start + page_size);

  res.json({
    documents: paged.map((d) => ({
      source_id: d.source_id,
      title: d.title,
      updated_at: d.updated_at,
    })),
    has_more: start + page_size < filtered.length,
    total: filtered.length,
  });
});

// ─── POST /documents/fetch ──────────────────────────────────────────────────
app.post("/documents/fetch", (req, res) => {
  const { source_ids } = req.body;
  if (!source_ids || !Array.isArray(source_ids)) {
    return res.status(400).json({ error: "source_ids array required" });
  }

  const results = source_ids
    .map((id) => documents.find((d) => d.source_id === id))
    .filter(Boolean);

  res.json({ documents: results });
});

// ─── Start server ───────────────────────────────────────────────────────────
app.listen(PORT, "0.0.0.0", () => {
  console.log(`Acme Wiki Connector running on port ${PORT}`);
  console.log(`  Health:  http://localhost:${PORT}/health`);
  console.log(`  Docs:    ${documents.length} documents available`);
});
