# Knowledge Intelligence

DocBrain's intelligence layer is built on five systems that transform it from a retrieval tool into a proactive knowledge engine: understanding knowledge structure, measuring documentation health, anticipating gaps, autonomously maintaining quality, and pushing intelligence to users before they ask.

---

## Knowledge Graph

The knowledge graph is DocBrain's map of your organization: entities (services, teams, concepts, people) and the relationships between them. The graph traversal engine moves through that map to answer structural questions that keyword or vector search cannot.

### What It Does

**BFS/DFS Traversal:** Breadth-first and depth-first traversal from any entity node. Used internally to resolve cross-document enrichment chains (PR → Jira → runbook → architecture doc) and exposed directly via the graph API for integration with other tools.

**Blast Radius Analysis:** Given an entity (a service, a configuration key, a team), compute which other entities would be affected if it changed or failed. Useful for change impact assessment and incident scoping.

**Expertise Routing:** Identify which team members have demonstrated knowledge of a given topic based on their document interactions and feedback history. Answers "who should I ask about X?" without requiring a maintained skills matrix.

**Entity Disambiguation:** When a query references "auth" or "the cache cluster", the graph resolves which entity is most likely meant based on context and relationship proximity.

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/v1/graph/entity/:name` | Look up an entity by name; returns type, relationships, and metadata |
| `GET /api/v1/graph/dependencies/:entity_id` | Traverse downstream dependencies of an entity |
| `GET /api/v1/graph/blast-radius/:entity_id` | Compute impact radius — which entities are downstream of this one |
| `GET /api/v1/graph/path` | Find the shortest path between two entities (`?from=&to=`) |
| `GET /api/v1/graph/experts/:topic` | Return users with demonstrated expertise on a topic |

### Configuration

Graph traversal uses the existing knowledge graph stored in Postgres (`entities` and `entity_relations` tables). No additional configuration is required. The traversal depth for blast radius analysis defaults to 3 hops and is not currently configurable via env var — adjust via the API query parameter `?depth=N` (max 5).

### Example Use Cases

- **Incident scoping:** `GET /api/v1/graph/blast-radius/auth-service` before a deploy to understand what's downstream.
- **Onboarding:** `GET /api/v1/graph/experts/kubernetes` to find who to shadow for a new platform team hire.
- **Architecture review:** `GET /api/v1/graph/path?from=payments-api&to=postgres-primary` to understand the dependency chain.

---

## Documentation Analytics

Documentation Analytics measures the health of your organization's documentation over time. Not a static score — a trend. It answers: is your team's collective knowledge getting more accessible or less? Are you accumulating tribal knowledge faster than you're documenting it?

### What It Does

**Daily snapshots:** A daily snapshot of documentation health is recorded automatically. Each snapshot captures gap resolution rate, knowledge half-life, tribal knowledge percentage, and ROI. Snapshots are taken during the memory consolidation cycle.

**Velocity computation:** Given a lookback window, computes:
- `gap_resolution_rate`: gaps resolved / gaps opened in the period
- `knowledge_half_life_days`: median time for a answered question to become a gap again
- `tribal_knowledge_pct`: percentage of answerable questions where only 1-2 people could have answered
- `roi_usd`: estimated dollar value of engineer time saved (configurable via `minutesSavedPerQuery` and `hourlyRate`)

**Tribal Knowledge %:** Derived from expertise routing data. When a question can only be answered by documents that a single person authored and no one else has engaged with, it's flagged as tribal knowledge. High tribal knowledge % is a risk indicator for bus-factor issues.

**Team Velocity Comparison:** Per-team breakdowns show which teams are improving documentation quality versus which are accumulating debt. Useful for engineering managers tracking documentation culture over time.

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/v1/analytics/velocity` | Current velocity metrics with trend vs. prior period |
| `GET /api/v1/analytics/velocity/teams` | Per-team velocity breakdown |

Query parameters:
- `?days=30` — lookback window (default 30, max 365)
- `?team=platform-eng` — filter to a specific team (teams endpoint only)

### Configuration

```bash
# In values.yaml or env vars:
VELOCITY_MINUTES_SAVED_PER_QUERY=15   # Assumed time saved per answered query (minutes)
VELOCITY_HOURLY_RATE=75               # Hourly engineer cost in USD for ROI calculation
```

### Example Use Cases

- **Quarterly review:** Pull 90-day velocity trend to present to engineering leadership as a documentation health KPI.
- **Team retrospective:** Compare team velocity to identify which teams are leading on documentation and which need support.
- **Hiring validation:** Track whether tribal knowledge % decreases as documentation investment increases.

---

## Predictive Intelligence

Predictive Intelligence detects documentation problems before users encounter them. Rather than waiting for a cluster of failed queries to surface a gap, it uses signal patterns to anticipate where gaps are about to form.

### What It Does

**Cascade staleness detection:** Identifies documents that are likely to become stale because they depend on other documents that have recently been updated. If "Redis Configuration Guide" references "Infrastructure Versions" and "Infrastructure Versions" is updated, "Redis Configuration Guide" is a cascade staleness candidate — even if users haven't complained about it yet.

**Seasonal forecasting:** Analyzes query history for seasonality. Some topics spike at predictable times: deployment procedures around release windows, onboarding docs in January, compliance procedures at quarter-end. DocBrain surfaces these before the spike, giving teams time to review and update relevant docs.

**Onboarding gap detection:** Identifies knowledge areas that new team members consistently struggle with in their first 30-60 days (inferred from query patterns of recently-created user accounts). Surfaces these proactively rather than waiting for the next cohort to hit the same wall.

**Code-change-triggered review:** When code changes are ingested (GitHub PR source), identifies runbooks, architecture docs, and how-to guides that likely need updating based on the semantic content of the diff.

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `POST /api/v1/predictive/code-change` | Trigger code-change gap analysis for a specific PR or commit |
| `GET /api/v1/predictive/cascade` | Return current cascade staleness candidates |
| `GET /api/v1/predictive/seasonal` | Return upcoming seasonal query spikes with affected docs |
| `GET /api/v1/predictive/onboarding` | Return onboarding gap analysis for recent user cohorts |

### Configuration

Predictive analysis runs during the memory consolidation cycle (default: every 6 hours). The code-change endpoint can be called from a CI/CD pipeline webhook to trigger immediate analysis on merge.

```bash
# Trigger from CI/CD (e.g. GitHub Actions on merge):
curl -X POST https://docbrain.internal/api/v1/predictive/code-change \
  -H "X-API-Key: db_sk_..." \
  -H "Content-Type: application/json" \
  -d '{"pr_url": "https://github.com/acme/platform/pull/1234", "diff_summary": "Updated Redis eviction config"}'
```

### Example Use Cases

- **Pre-release checklist:** Run `GET /api/v1/predictive/cascade` before a major release to find docs that might need updating.
- **Seasonal preparation:** Two weeks before a quarter-end compliance spike, review the seasonal analysis and proactively update relevant policies.
- **Onboarding improvement:** Review onboarding gaps before each new hire cohort starts.

---

## Autonomous Document Maintenance

Autonomous Maintenance closes the loop on documentation quality without requiring a quarterly audit. DocBrain continuously proposes targeted fixes — contradictions, broken links, outdated version references — and presents them to an admin for one-click approval.

### What It Does

**Contradiction fixes:** When the freshness scorer detects a contradiction between two documents (e.g., two runbooks that give conflicting instructions for the same procedure), DocBrain generates a specific fix proposal: which document should be authoritative, what language change would resolve the contradiction, and a confidence score for the proposal.

**Link repairs:** Detects broken internal links (links to Confluence pages that no longer exist, GitHub PRs that were closed without documentation updates, Jira tickets that were resolved months ago but are still cited as "in progress"). Proposes either updated links or removal of stale references.

**Version updates:** Identifies version numbers in documentation that are likely outdated (e.g., a tutorial that references `kubectl v1.24` when your cluster is on `v1.30`). Proposes specific version updates with links to the authoritative source.

**Approval workflow:** An admin reviews proposed fixes in the maintenance dashboard and applies or rejects each one. Applied fixes are committed back to the source document (Confluence page update, etc.) and the document is re-ingested. Rejected fixes are recorded with optional feedback to improve future proposals.

**Cross-document consistency check:** Runs a broader consistency sweep across related document clusters (e.g., all documents about "payments service") to surface subtle inconsistencies that don't rise to the level of contradiction but represent documentation drift.

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/v1/maintenance/fixes` | List pending fix proposals (paginated, filterable by type/severity) |
| `POST /api/v1/maintenance/fixes/:id/apply` | Apply a fix proposal; triggers document update and re-ingest |
| `POST /api/v1/maintenance/fixes/:id/reject` | Reject a fix proposal with optional feedback |
| `GET /api/v1/maintenance/stats` | Summary: pending fixes by type, applied this week, auto-resolved |

Query parameters for the list endpoint:
- `?type=contradiction|link|version` — filter by fix type
- `?status=pending|applied|rejected` — filter by status
- `?severity=high|medium|low` — filter by severity

### Configuration

```bash
# How many contradiction checks per freshness pass (higher = more LLM cost):
FRESHNESS_CONTRADICTION_CHECKS_PER_PASS=10

# Include recently ingested event docs (Slack, PRs) in contradiction checks:
FRESHNESS_CONTRADICTION_INCLUDE_RECENT_EVENT_DOCS=true

# Max age for event docs to be included in contradiction checks:
FRESHNESS_CONTRADICTION_EVENT_DOC_MAX_AGE_DAYS=90
```

### Example Use Cases

- **Weekly review:** `GET /api/v1/maintenance/fixes?status=pending` in the admin UI to review and apply that week's proposals.
- **Post-migration cleanup:** After a major infrastructure migration, run `cross_doc_consistency_check` to find all docs that still reference old system names or configurations.
- **Automated low-risk fixes:** Set a policy to auto-apply `link_repair` fixes with confidence > 0.95 (API automation from a scheduled job).

---

## Knowledge Stream

Knowledge Stream shifts DocBrain from reactive (answering questions) to proactive (pushing intelligence). It continuously monitors signals and delivers targeted alerts to the right people before they need to ask.

### What It Does

**Incident early warnings:** Monitors the pattern of queries in real-time. When multiple users start asking similar questions within a short window (default: ≥2 unique users within 2 hours), it infers a possible incident in progress and fires an early warning. This can catch incidents via documentation access patterns before monitoring alerts fire.

**Knowledge decay risk:** Identifies documents that are highly accessed but haven't been updated recently. High traffic on a stale document is a risk indicator — the team is relying on it, but it may be wrong. Sends proactive alerts to document authors.

**Expertise gap detection:** Monitors expert activity. When a recognized domain expert (identified via expertise routing) hasn't interacted with DocBrain in more than the configured threshold (default: 90 days), their knowledge areas are flagged as at-risk single points of failure.

**Context-aware author notifications:** Combines signals from multiple sources (freshness scores, query patterns, code changes, expertise gaps) to generate context-aware recommendations for document authors. Rather than generic "this doc is stale" notifications, authors receive: "Your 'Redis Configuration' doc is being accessed by 3x its normal volume, hasn't been updated in 8 months, and 2 recent PRs have changed the Redis configuration. Here's what might need updating."

### API Endpoints

| Endpoint | Description |
|----------|-------------|
| `GET /api/v1/stream/events` | List stream events (paginated, filterable by type/severity) |
| `GET /api/v1/stream/events/user/:user_id` | Events relevant to a specific user (for personalized feeds) |
| `POST /api/v1/stream/context` | Submit context for real-time analysis (e.g., from a Slack webhook) |
| `GET /api/v1/stream/stats` | Summary: events generated, acknowledged, acted on (last 7/30 days) |

### Configuration

```yaml
# values.yaml
stream:
  enabled: false                 # Opt-in; set true to enable proactive alerting
  intervalMinutes: 30            # Scan frequency
  incidentWarningMinUsers: 2     # Min unique users within 2h for incident warning
  expertiseGapDays: 90           # Days of expert inactivity before gap alert
  # alertChannel: ""             # Optional Slack channel for critical alerts
```

```bash
# Equivalent env vars:
STREAM_ENABLED=true
STREAM_INTERVAL_MINUTES=30
STREAM_INCIDENT_WARNING_MIN_USERS=2
STREAM_EXPERTISE_GAP_DAYS=90
STREAM_ALERT_CHANNEL=#docbrain-alerts
```

### Example Use Cases

- **Incident detection:** Route `incident_early_warning` events to your on-call Slack channel. DocBrain may surface a brewing incident before PagerDuty fires.
- **Author notifications:** Subscribe document authors to their personalized stream (`/stream/events/user/:id`) via a weekly digest email or Slack DM.
- **Expertise coverage planning:** Use `expertise_gap` events during team capacity planning to identify knowledge transfer priorities before an expert goes on leave.
