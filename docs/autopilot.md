# Autopilot — Gap Analysis & Doc Drafting

Autopilot is DocBrain's feedback-loop engine. It continuously monitors Q&A history, identifies documentation gaps, generates draft content using all 5 memory layers, and closes the loop by publishing docs back to your knowledge base.

---

## How It Works

```
User asks question
        │
        ▼
DocBrain answers (with confidence score)
        │
        ├─ Positive feedback ──► episode stored, memory strengthened
        │
        └─ Negative feedback ──► episode flagged, negative_count++
                                          │
                              ┌───────────▼────────────┐
                              │  Gap Analyzer (6h cron) │
                              │  - clusters by embedding│
                              │  - filters by thresholds│
                              │  - scores by severity   │
                              └───────────┬────────────┘
                                          │
                              ┌───────────▼────────────┐
                              │  Doc Drafter            │
                              │  - pulls episodic notes │
                              │  - queries KG for facts │
                              │  - applies freshness    │
                              └───────────┬────────────┘
                                          │
                              ┌───────────▼────────────┐
                              │  Publisher              │
                              │  - UPDATE existing page │
                              │    (poor_coverage gaps) │
                              │  - CREATE new page      │
                              │    (missing_doc gaps)   │
                              └───────────┬────────────┘
                                          │
                              Gap cluster marked resolved
                              Page re-ingested on next cycle
```

---

## Gap Types

| Type | What it means | Publish action |
|---|---|---|
| `poor_coverage` | DocBrain has a doc but it's incomplete or stale. Users repeatedly get low-confidence answers from it. | **UPDATE** the existing Confluence page |
| `missing_doc` | No relevant documentation exists at all. DocBrain returned no meaningful results. | **CREATE** a new Confluence page |

For `poor_coverage` gaps the UI shows "Updates existing doc: [url]". For `missing_doc` gaps it shows "Creates new documentation page".

---

## Severity Scoring

Every gap cluster gets a composite severity score (0–1) from six components:

```
composite =
  0.25 × breadth_score     (how many unique users hit this gap)
  0.25 × volume_score      (how many negative signals total)
  0.20 × ratio_score       (fraction of topic queries that are negative)
  0.15 × confidence_score  (how confused DocBrain was — low confidence = high score)
  0.10 × recency_score     (how recently the gap was hit)
  0.05 × signal_severity   (how many signals are hard failures: "incorrect"/"not_relevant")
```

The composite is then mapped to a severity band:

| Severity | Default threshold | Meaning |
|---|---|---|
| **critical** | ≥ 0.75 | Multiple users, high volume, recent. Fix immediately. |
| **high** | ≥ 0.55 | Moderate breadth/volume. Fix this week. |
| **medium** | ≥ 0.35 | Small signal. Monitor. |
| **low** | < 0.35 | Minimal evidence. Not actionable yet. |

---

## Configuration Reference

### Clustering

| Variable | Default | Description |
|---|---|---|
| `AUTOPILOT_CLUSTER_THRESHOLD` | `0.82` | Cosine similarity to join two queries into the same cluster. Higher = tighter, more specific clusters. Lower = broader, catch-all clusters. |
| `AUTOPILOT_MIN_CLUSTER_SIZE` | `3` | Minimum episode count for a cluster to surface as a gap. |
| `AUTOPILOT_MIN_UNIQUE_USERS` | `2` | Minimum distinct users that must contribute to the cluster. Guards against one user flooding the signal. |
| `AUTOPILOT_MIN_NEGATIVE_RATIO` | `0.15` | Minimum fraction of cluster queries with negative feedback. Filters noise. |
| `AUTOPILOT_LOOKBACK_DAYS` | `30` | How far back in time to scan for episodes. |
| `AUTOPILOT_MAX_CLUSTERS` | `50` | Max gap clusters to persist per run. Caps DB write volume. |
| `AUTOPILOT_MAX_EPISODES` | `500` | Max episodes to load per run. Caps memory usage. |
| `AUTOPILOT_GAP_ANALYSIS_INTERVAL_HOURS` | `6` | How often the background scheduler runs. |

### Severity Scoring Scale Factors

These control when a gap escalates to "critical" or "high". All are configurable without code changes.

| Variable | Default | Description |
|---|---|---|
| `AUTOPILOT_CRITICAL_USERS` | `5` | Unique users needed to give breadth_score = 1.0. |
| `AUTOPILOT_CRITICAL_SIGNALS` | `15` | Negative signals needed to give volume_score = 1.0. |
| `AUTOPILOT_CRITICAL_THRESHOLD` | `0.75` | Composite score cutoff for **critical**. |
| `AUTOPILOT_HIGH_THRESHOLD` | `0.55` | Composite score cutoff for **high**. |
| `AUTOPILOT_MEDIUM_THRESHOLD` | `0.35` | Composite score cutoff for **medium**. |

### Auto-Draft

| Variable | Default | Description |
|---|---|---|
| `AUTOPILOT_AUTO_DRAFT` | `false` | When `true`, drafts are generated automatically after each analysis run without human trigger. |
| `AUTOPILOT_AUTO_DRAFT_SEVERITY` | `critical` | Minimum severity to auto-draft. |

---

## Tuning for Your Org Size

### Small team / dev environment (< 10 users)

With few users, the default thresholds (5 users, 15 signals) mean you'll rarely see critical gaps. Lower the scale factors:

```bash
# .env or helm values
AUTOPILOT_CRITICAL_USERS=1          # Any single user can drive breadth to 1.0
AUTOPILOT_CRITICAL_SIGNALS=3        # 3 negative signals is "high volume" for you
AUTOPILOT_CRITICAL_THRESHOLD=0.30   # Easy to hit critical
AUTOPILOT_HIGH_THRESHOLD=0.20
AUTOPILOT_MEDIUM_THRESHOLD=0.10

# Also relax clustering filters
AUTOPILOT_MIN_CLUSTER_SIZE=1
AUTOPILOT_MIN_UNIQUE_USERS=1
AUTOPILOT_MIN_NEGATIVE_RATIO=0.05
```

Helm equivalent:
```yaml
autopilot:
  criticalUsers: 1
  criticalSignals: 3
  thresholdCritical: 0.30
  thresholdHigh: 0.20
  thresholdMedium: 0.10
  minClusterSize: 1
  minUniqueUsers: 1
  minNegativeRatio: 0.05
```

### Medium team (10–100 users)

Balanced defaults work well. Fine-tune if you're getting too many or too few gaps:

```bash
AUTOPILOT_CRITICAL_USERS=5           # Default
AUTOPILOT_CRITICAL_SIGNALS=15        # Default
AUTOPILOT_CRITICAL_THRESHOLD=0.65    # Slightly lower than default to see more criticals
AUTOPILOT_MIN_CLUSTER_SIZE=2
AUTOPILOT_MIN_UNIQUE_USERS=2
```

### Large org (100+ users)

Raise the bar so only truly widespread gaps reach critical:

```bash
AUTOPILOT_CRITICAL_USERS=20          # Need 20 distinct users to reach full breadth
AUTOPILOT_CRITICAL_SIGNALS=50        # 50 negatives for full volume
AUTOPILOT_CRITICAL_THRESHOLD=0.75    # Keep default threshold
AUTOPILOT_MIN_UNIQUE_USERS=5
AUTOPILOT_MIN_CLUSTER_SIZE=10
AUTOPILOT_MIN_NEGATIVE_RATIO=0.20
```

---

## Which Variables Give the Highest Results?

"Highest results" means more gaps surfaced at higher severities. The most impactful levers, in order:

1. **`AUTOPILOT_CRITICAL_USERS`** (highest impact) — drives 25% of composite score. Set to 1 and every anonymous gap can hit critical.
2. **`AUTOPILOT_CRITICAL_SIGNALS`** (high impact) — drives another 25%. Set to 3 and a few complaints = full volume score.
3. **`AUTOPILOT_CRITICAL_THRESHOLD`** — lowers the bar for the "critical" label. Dropping from 0.75 to 0.30 will immediately promote existing "high" gaps to critical.
4. **`AUTOPILOT_MIN_CLUSTER_SIZE=1`** + **`AUTOPILOT_MIN_UNIQUE_USERS=1`** — these are gate filters that prevent gaps from appearing at all. Setting both to 1 means every single negative interaction can become a visible gap cluster.
5. **`AUTOPILOT_MIN_NEGATIVE_RATIO=0.05`** — only 5% of queries on a topic need to be negative (vs 15% default).

For a "see everything" dev config:
```bash
AUTOPILOT_CRITICAL_USERS=1
AUTOPILOT_CRITICAL_SIGNALS=3
AUTOPILOT_CRITICAL_THRESHOLD=0.20
AUTOPILOT_HIGH_THRESHOLD=0.12
AUTOPILOT_MEDIUM_THRESHOLD=0.05
AUTOPILOT_MIN_CLUSTER_SIZE=1
AUTOPILOT_MIN_UNIQUE_USERS=1
AUTOPILOT_MIN_NEGATIVE_RATIO=0.01
```

---

## GitLab MR Ingestion

GitLab Merge Requests can be captured as context for Autopilot gap analysis. When a relevant MR is merged (e.g., fixing a bug that caused user confusion), the MR description, comments, and diff summary are ingested as episodic memory.

**Setup steps:**

1. Go to your GitLab project → **Settings → Webhooks**
2. Add a webhook pointing to:
   ```
   POST https://<your-docbrain-host>/api/v1/capture/gitlab
   ```
3. Select trigger: **Merge request events**
4. Set the secret token and configure in DocBrain:
   ```bash
   GITLAB_WEBHOOK_SECRET=<your-secret>
   ```
5. Merged MRs will automatically appear as episodes tagged `source: gitlab_mr`

**What gets captured:**
- MR title and description (markdown)
- Labels (mapped to feedback signals)
- Merge author and reviewer context
- MR URL stored as `source_url` for traceability

**Helm:**
```yaml
gitlab:
  webhookSecret: ""   # set via secret in production
```

---

## Draft Publishing to Confluence

### Cloud (v2 API)

```bash
CONFLUENCE_BASE_URL=https://your-org.atlassian.net/wiki
CONFLUENCE_USER_EMAIL=bot@your-org.com
CONFLUENCE_API_TOKEN=<api-token-from-atlassian>
CONFLUENCE_API_VERSION=v2
DRAFT_PUBLISH_TARGET=confluence
DRAFT_PUBLISH_CONFLUENCE_SPACE_KEY=ENG
DRAFT_PUBLISH_AUTO_INGEST=true
```

### Data Center (v1 API)

```bash
CONFLUENCE_BASE_URL=https://confluence.internal/wiki
CONFLUENCE_API_TOKEN=<personal-access-token>
CONFLUENCE_API_VERSION=v1
DRAFT_PUBLISH_TARGET=confluence
DRAFT_PUBLISH_CONFLUENCE_SPACE_KEY=ENG
```

### How UPDATE vs CREATE works

- **`poor_coverage` gaps**: DocBrain finds the most-retrieved document for that cluster and stores its URL in `existing_doc_url`. On publish, it GETs the current page version, increments it, and PUTs the AI-enhanced content back. The footer reads _"AI-Enhanced Documentation"_.
- **`missing_doc` gaps**: No existing doc is identified. DocBrain CREATEs a new page under the configured parent. The footer reads _"AI-Generated Documentation"_.

---

## Helm Chart Reference

All autopilot settings are in `values.yaml` under the `autopilot:` key:

```yaml
autopilot:
  enabled: true
  lookbackDays: 30
  maxClusters: 50
  maxEpisodes: 500
  gapAnalysisIntervalHours: 6
  autoDraft: false
  autoDraftSeverity: critical   # critical | high | medium | low

  # Severity scale factors — tune for org size
  criticalUsers: 5        # env: AUTOPILOT_CRITICAL_USERS
  criticalSignals: 15     # env: AUTOPILOT_CRITICAL_SIGNALS
  thresholdCritical: 0.75 # env: AUTOPILOT_CRITICAL_THRESHOLD
  thresholdHigh: 0.55     # env: AUTOPILOT_HIGH_THRESHOLD
  thresholdMedium: 0.35   # env: AUTOPILOT_MEDIUM_THRESHOLD
```

All values map 1:1 to environment variables via the Helm `configmap.yaml` template.

---

## Monitoring & Observability

Key metrics to watch after changing thresholds:

- **`/api/v1/autopilot/gaps`** — count of gaps by severity. After lowering thresholds, expect more `critical` and `high` entries.
- **`/api/v1/autopilot/summary`** — aggregate counts used by the homepage dashboard.
- **Gap cluster `composite_score`** — returned in the gap list API. Useful for understanding exactly why a gap got its severity level.

To inspect gap scoring manually:
```sql
SELECT topic_label, severity, composite_score, unique_user_count, negative_count, gap_type
FROM autopilot_gap_clusters
ORDER BY composite_score DESC
LIMIT 20;
```

---

## FAQ

**Q: Why do I see 0 critical gaps on the homepage?**

The composite score must reach `AUTOPILOT_CRITICAL_THRESHOLD` (default 0.75). With few users or signals (typical in dev/staging), scores rarely reach that level. Lower `AUTOPILOT_CRITICAL_USERS` and `AUTOPILOT_CRITICAL_SIGNALS` — or lower the threshold itself — to surface gaps at your signal volume.

**Q: My gap clusters have `unique_user_count=0`. Why?**

Episodes ingested before user_id tracking was enabled (or from anonymous API keys) have no user_id. Autopilot uses query text diversity as a proxy: if 5 distinct query texts hit the same cluster, that counts as 5 for breadth scoring purposes.

**Q: How do I stop autopilot from creating too many low-quality drafts?**

Raise `AUTOPILOT_AUTO_DRAFT_SEVERITY` to `critical` (default) and increase `AUTOPILOT_MIN_CLUSTER_SIZE` and `AUTOPILOT_MIN_UNIQUE_USERS`. This ensures only gaps backed by multiple real users generate auto-drafts.

**Q: Can I trigger gap analysis manually?**

Yes:
```
POST /api/v1/autopilot/analyze   — Admin only
```

**Q: How do I re-run autopilot for a specific lookback window?**

Temporarily override `AUTOPILOT_LOOKBACK_DAYS` and POST to `/api/v1/autopilot/analyze`. The env var takes effect immediately without restart.
