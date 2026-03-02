# Configuration Reference

## How Configuration Works

DocBrain uses a **config-first architecture** with a layered YAML + environment variable system. Understanding this prevents confusion about why a value isn't taking effect.

### Loading Order (later = higher priority)

```
config/default.yaml         ← committed to repo — all non-secret defaults
config/{APP_ENV}.yaml       ← environment-specific overrides (development | production)
config/local.yaml           ← gitignored — your secrets and local overrides
Environment variables / .env ← always win — highest priority
```

Set `APP_ENV=production` for the production profile (this is the default in the Docker image). The server defaults to `APP_ENV=development` when running locally without Docker.

### What Goes Where

| Type | Where to put it |
|---|---|
| Infrastructure secrets (DB URL, LLM API keys, Redis, OpenSearch) | `.env` or environment variables |
| Ingest source credentials (Confluence token, GitHub token, Slack token, Jira token) | `config/local.yaml` (gitignored) |
| Deployment-specific values (URLs, ports, CORS origins) | `.env` or environment variables |
| Tuning (thresholds, intervals, cache TTLs) | `config/local.yaml` or env vars |
| Team-wide defaults you want committed | `config/default.yaml` (no secrets!) |

**The key distinction:** `.env` is for infrastructure secrets that the runtime environment must inject (container orchestration, CI/CD, secrets managers). `config/local.yaml` is for user-managed source credentials and personal overrides — it's gitignored so it never gets committed, but it lives alongside the project where you can edit it easily.

### Example `config/local.yaml`

```yaml
# config/local.yaml — never committed (gitignored)
# Configure ingest sources and personal overrides here.

ingest:
  ingest_sources: confluence,github_pr

confluence:
  base_url: https://acme.atlassian.net/wiki
  user_email: you@acme.com
  api_token: ATATT3x...
  space_keys: DOCS,ENG

github_pr:
  token: ghp_...
  repo: acme/platform
  lookback_days: 180

# Local tuning overrides (optional)
autopilot:
  enabled: true
  cluster_threshold: 0.78

rag:
  cache_ttl_hours: 1
```

### YAML Config Structure

Every YAML value supports `${ENV_VAR}` and `${ENV_VAR:-default}` substitution:

```yaml
database:
  url: "${DATABASE_URL}"     # required — must come from env
  max_connections: "${DB_MAX_CONNECTIONS:-10}"
```

### Custom Config Directory

```bash
# Mount a ConfigMap in Kubernetes
DOCBRAIN_CONFIG_DIR=/etc/docbrain docbrain-server

# Or pass as CLI argument
docbrain-server --config-dir /etc/docbrain
```

---

All configuration is also available via environment variables, set in `.env` for Docker Compose or via ConfigMap/Secret for Kubernetes. **Environment variables always override YAML values.**

## Infrastructure

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | — | PostgreSQL connection string |
| `OPENSEARCH_URL` | `http://localhost:9200` | OpenSearch endpoint |
| `REDIS_URL` | `redis://localhost:6379` | Redis connection string |
| `SERVER_PORT` | `3000` | API server listen port |
| `SERVER_BIND` | `0.0.0.0` | API server bind address |
| `LOG_LEVEL` | `info` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |
| `DB_MAX_CONNECTIONS` | `10` | Maximum PostgreSQL connection pool size |
| `DB_CONNECT_TIMEOUT_SECS` | `10` | Timeout (seconds) for initial PostgreSQL connection |
| `DB_ACQUIRE_TIMEOUT_SECS` | `10` | Timeout (seconds) to acquire a connection from the pool |
| `DB_IDLE_TIMEOUT_SECS` | `300` | Idle connection lifetime (seconds) before cleanup |

## LLM Provider

| Variable | Default | Description |
|----------|---------|-------------|
| `LLM_PROVIDER` | `bedrock` | Provider: `bedrock`, `anthropic`, `openai`, `ollama` |
| `LLM_MODEL_ID` | varies | Model identifier (provider-specific) |
| `LLM_THINKING_BUDGET` | `10000` | Max thinking tokens for extended thinking models |
| `ANTHROPIC_API_KEY` | — | API key (if `LLM_PROVIDER=anthropic`) |
| `OPENAI_API_KEY` | — | API key (if `LLM_PROVIDER=openai`) |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama server URL |
| `OLLAMA_TLS_VERIFY` | `false` | Set to `true` to enforce TLS certificate validation for Ollama |
| `OLLAMA_VISION_ENABLED` | `true` | Set to `false` if your Ollama model doesn't support vision (skips image calls) |
| `AWS_REGION` | — | AWS region (if `LLM_PROVIDER=bedrock`) |

## Embedding Provider

| Variable | Default | Description |
|----------|---------|-------------|
| `EMBED_PROVIDER` | `bedrock` | Provider: `bedrock`, `openai`, `ollama` |
| `EMBED_MODEL_ID` | varies | Embedding model identifier |

## Document Ingestion

Configure sources in `config/local.yaml` (gitignored). Put only infrastructure secrets in `.env`.

### General

| Setting (`config/local.yaml` key) | Env var equivalent | Default | Description |
|---|---|---|---|
| `ingest.ingest_sources` | `INGEST_SOURCES` | `local` | Comma-separated list of active sources: `local`, `confluence`, `github`, `github_pr`, `gitlab_mr`, `slack_thread`, `jira` |
| `ingest.self_ingest` | `DOCBRAIN_SELF_INGEST` | `true` | Auto-ingest DocBrain's own docs |
| `ingest.image_extraction_enabled` | `IMAGE_EXTRACTION_ENABLED` | `true` | Extract and describe images using vision LLM |

### Local Files

| Variable | Default | Description |
|----------|---------|-------------|
| `LOCAL_DOCS_PATH` | — | Directory path for local file ingestion (set in `.env` or as env var) |

### Confluence

Set credentials in `config/local.yaml`:

```yaml
confluence:
  base_url: https://yourco.atlassian.net/wiki
  user_email: you@yourco.com
  api_token: ATATT3x...
  space_keys: ENG,DOCS
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `confluence.base_url` | `CONFLUENCE_BASE_URL` | — | Atlassian instance URL (must include `/wiki`) |
| `confluence.user_email` | `CONFLUENCE_USER_EMAIL` | — | Auth email (not required for v1 Data Center) |
| `confluence.api_token` | `CONFLUENCE_API_TOKEN` | — | API token (Cloud) or Personal Access Token (Data Center) |
| `confluence.space_keys` | `CONFLUENCE_SPACE_KEYS` | — | Comma-separated space keys to ingest |
| `confluence.page_limit` | `CONFLUENCE_PAGE_LIMIT` | `0` (unlimited) | Max pages per space. `0` = all pages. |
| `confluence.api_version` | `CONFLUENCE_API_VERSION` | `v2` | `v2` for Cloud, `v1` for Data Center 7.x+ |
| `confluence.tls_verify` | `CONFLUENCE_TLS_VERIFY` | `true` | Set to `false` for self-signed certs |
| `confluence.webhook_secret` | `CONFLUENCE_WEBHOOK_SECRET` | — | HMAC secret for real-time webhook sync (set as env var) |

### GitHub Repository

```yaml
# config/local.yaml
github:
  repo_url: https://github.com/your-org/your-docs
  token: ghp_...    # only for private repos
  branch: main
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `github.repo_url` | `GITHUB_REPO_URL` | — | Repository URL to clone and ingest |
| `github.token` | `GITHUB_TOKEN` | — | Personal access token (optional for public repos) |
| `github.branch` | `GITHUB_BRANCH` | `main` | Branch to ingest from |

### GitHub Pull Requests

Ingest PR titles, descriptions, and review discussions as searchable knowledge.

```yaml
# config/local.yaml
github_pr:
  token: ghp_...
  repo: acme/platform
  lookback_days: 365
  min_comments: 1
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `github_pr.token` | `GITHUB_PR_TOKEN` | — | GitHub personal access token (secret — set in `config/local.yaml`) |
| `github_pr.repo` | `GITHUB_PR_REPO` | — | Owner/repo (e.g. `acme/platform`) — set in `config/local.yaml` |
| `github_pr.lookback_days` | `GITHUB_PR_LOOKBACK_DAYS` | `365` | How far back to fetch PRs |
| `github_pr.min_comments` | `GITHUB_PR_MIN_COMMENTS` | `1` | Minimum comments for a PR to be ingested |
| `github_pr.labels` | `GITHUB_PR_LABELS` | — | Comma-separated label filter (optional) |
| `github_pr.api_url` | `GITHUB_PR_API_URL` | — | Override for GitHub Enterprise (optional) |

### GitLab Merge Requests

Ingest MR titles, descriptions, and discussion threads.

```yaml
# config/local.yaml
gitlab_mr:
  token: glpat-...
  project_ids: acme/platform,acme/infra
  lookback_days: 365
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `gitlab_mr.token` | `GITLAB_TOKEN` | — | GitLab personal access token (secret — set in `config/local.yaml`) |
| `gitlab_mr.base_url` | `GITLAB_BASE_URL` | `https://gitlab.com` | GitLab instance URL |
| `gitlab_mr.project_ids` | `GITLAB_PROJECT_IDS` | — | Comma-separated namespace/repo paths — set in `config/local.yaml` |
| `gitlab_mr.lookback_days` | `GITLAB_MR_LOOKBACK_DAYS` | `365` | How far back to fetch MRs |
| `gitlab_mr.min_notes` | `GITLAB_MR_MIN_NOTES` | `1` | Minimum notes/comments for an MR to be ingested |
| `gitlab_mr.labels` | `GITLAB_MR_LABELS` | — | Comma-separated label filter (optional) |
| `gitlab_mr.tls_verify` | `GITLAB_TLS_VERIFY` | `true` | Set to `false` for self-signed certs |

### Slack Threads

Ingest high-signal Slack threads (by reaction count or reply threshold).

```yaml
# config/local.yaml
slack_ingest:
  token: xoxb-...
  channels: C01234567,C09876543
  min_replies: 3
  reactions: "white_check_mark,bookmark"
  lookback_days: 90
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `slack_ingest.token` | `SLACK_INGEST_TOKEN` | — | Slack bot token (secret — set in `config/local.yaml`) |
| `slack_ingest.channels` | `SLACK_INGEST_CHANNELS` | — | Comma-separated channel IDs — set in `config/local.yaml` |
| `slack_ingest.min_replies` | `SLACK_MIN_REPLIES` | `3` | Minimum thread replies to be ingested |
| `slack_ingest.reactions` | `SLACK_INGEST_REACTIONS` | `white_check_mark,bookmark` | Comma-separated reaction names that flag a thread for ingest |
| `slack_ingest.lookback_days` | `SLACK_LOOKBACK_DAYS` | `90` | How far back to scan channels |

### Jira

Ingest Jira issues (bugs, stories, tasks, epics) as searchable knowledge.

```yaml
# config/local.yaml
jira_ingest:
  base_url: https://yourcompany.atlassian.net
  user_email: you@yourcompany.com
  api_token: your-token
  projects: ENG,OPS
  lookback_days: 365
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `jira_ingest.base_url` | `JIRA_BASE_URL` | — | Jira instance URL — set in `config/local.yaml` |
| `jira_ingest.user_email` | `JIRA_USER_EMAIL` | — | Jira account email — set in `config/local.yaml` |
| `jira_ingest.api_token` | `JIRA_API_TOKEN` | — | Jira API token (secret — set in `config/local.yaml`) |
| `jira_ingest.projects` | `JIRA_PROJECTS` | — | Comma-separated project keys — set in `config/local.yaml` |
| `jira_ingest.jql_filter` | `JIRA_JQL_FILTER` | — | Additional JQL filter (optional) |
| `jira_ingest.lookback_days` | `JIRA_LOOKBACK_DAYS` | `365` | How far back to fetch issues |
| `jira_ingest.issue_types` | `JIRA_ISSUE_TYPES` | `Bug,Story,Task,Epic` | Comma-separated issue types to ingest |

## Confluence Webhooks (Real-Time Sync)

| Variable | Default | Description |
|----------|---------|-------------|
| `CONFLUENCE_WEBHOOK_SECRET` | — | HMAC secret shared with Confluence. When set, DocBrain mounts `POST /confluence/events` and auto-ingests page changes in real time. Set as an environment variable (not in `config/local.yaml`). |

When configured, DocBrain receives `page_created`, `page_updated`, `page_restored`, `page_removed`, and `page_trashed` events from Confluence and syncs changes automatically — no scheduled re-ingest needed.

Requires `confluence.base_url` and `confluence.api_token` to also be set in `config/local.yaml` (DocBrain needs API access to fetch the page content when a webhook fires).

See the [Ingestion Guide](ingestion.md#real-time-sync-confluence-webhooks) for setup instructions.

## Image Extraction

| Variable | Default | Description |
|----------|---------|-------------|
| `IMAGE_EXTRACTION_ENABLED` | `true` | Extract and describe images from Confluence pages using vision LLM. Set to `false` to disable. |
| `HAIKU_MODEL_ID` | — | Model used for image descriptions (cheaper/faster). Falls back to `LLM_MODEL_ID` if not set. |
| `IMAGE_MAX_PER_PAGE` | `20` | Maximum images to process per Confluence page |
| `IMAGE_MIN_SIZE_BYTES` | `5120` | Skip images smaller than this in bytes (default: 5 KB) — filters out icons and decorative images |
| `IMAGE_MAX_SIZE_BYTES` | `10485760` | Skip images larger than this in bytes (default: 10 MB) |
| `IMAGE_DOWNLOAD_TIMEOUT` | `30` | HTTP download timeout in seconds per image |
| `IMAGE_LLM_TIMEOUT` | `120` | LLM vision call timeout in seconds (needs more time than download) |

Image extraction requires a vision-capable LLM. Supported providers: **Bedrock**, **Anthropic**, **OpenAI**, and **Ollama** (with vision models like `llava`, `llama3.2-vision`, `moondream`). Text-only models (e.g. `llama3.1`) are auto-detected and images are skipped gracefully — no failures, no errors.

## Web UI / CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3001` | Comma-separated origins allowed to call the API. Only needed if the web UI is served from a non-default origin (e.g. `http://10.0.0.5:3001`, `https://docbrain.internal`) |

> **Note:** The default works out of the box for Docker Compose. You only need this if you access the web UI via a different hostname or port — for example, `http://127.0.0.1:3001` is a different origin than `http://localhost:3001`.

## Auth / Sessions

| Variable | Default | Description |
|----------|---------|-------------|
| `LOGIN_SESSION_TTL_HOURS` | `720` | Session lifetime after email/password login (default: 720 hours = 30 days). Set to `0` for no expiry. |
| `MAX_QUERY_LENGTH` | `4000` | Maximum characters allowed for question and description inputs |

## Slack Integration (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `SLACK_BOT_TOKEN` | — | Slack bot OAuth token (`xoxb-...`) |
| `SLACK_SIGNING_SECRET` | — | Slack app signing secret |
| `SLACK_GAP_NOTIFICATION_CHANNEL` | — | Channel to post critical gap alerts after each analysis run (e.g. `#docs-alerts`). Only fires when new critical-severity gaps are found. Requires `SLACK_BOT_TOKEN`. |

## Notifications (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `NOTIFICATION_INTERVAL_HOURS` | `24` | How often to check for stale docs and send owner DMs |
| `NOTIFICATION_SPACE_FILTER` | — | Comma-separated spaces to limit notifications (e.g. `PLATFORM,SRE`). Empty = all spaces. |

## Documentation Autopilot (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `AUTOPILOT_ENABLED` | `false` | Enable the Documentation Autopilot (gap detection + draft generation) |
| `AUTOPILOT_GAP_ANALYSIS_INTERVAL_HOURS` | `6` | How often the background scheduler runs gap analysis |
| `AUTOPILOT_LOOKBACK_DAYS` | `30` | Days of query history to analyse for gaps |
| `AUTOPILOT_CLUSTER_THRESHOLD` | `0.82` | Cosine similarity threshold for grouping queries into a gap cluster (0.65 = loose, 0.85 = strict) |
| `AUTOPILOT_MIN_CLUSTER_SIZE` | `3` | Minimum episodes in a cluster to be considered a real gap |
| `AUTOPILOT_MIN_UNIQUE_USERS` | `2` | Minimum distinct users that must hit the same gap topic |
| `AUTOPILOT_MIN_NEGATIVE_RATIO` | `0.15` | Minimum fraction of queries on a topic that must have negative feedback |
| `AUTOPILOT_MAX_CLUSTERS` | `50` | Maximum gap clusters to persist per analysis run |
| `AUTOPILOT_MAX_EPISODES` | `500` | Maximum negative episodes to load per analysis run |

When enabled, Autopilot runs on the configured schedule, exposes management endpoints at `/api/v1/autopilot/*`, and posts critical gap alerts to `SLACK_GAP_NOTIFICATION_CHANNEL` if configured. See the [API Reference](api-reference.md) for endpoint details.

## Freshness Scoring

| Variable | Default | Description |
|----------|---------|-------------|
| `FRESHNESS_SCHEDULER_INTERVAL_HOURS` | `24` | How often freshness scores are recalculated for all documents |

## Memory Consolidation

| Variable | Default | Description |
|----------|---------|-------------|
| `CONSOLIDATION_INTERVAL_HOURS` | `6` | How often the memory consolidation job runs (merges episodic patterns into semantic/procedural memory) |

## RAG Pipeline

| Variable | Default | Description |
|----------|---------|-------------|
| `RAG_TOP_K` | `10` | Chunks retrieved per query. Higher = more context passed to the LLM, at the cost of more tokens per call. Raise to `15`–`20` if answers are missing obvious information; lower to `5` to reduce cost on simple corpora. |
| `RAG_BM25_BOOST` | `1.0` | Weight of keyword (BM25) search relative to vector search in hybrid retrieval. Raise to `2.0`–`3.0` for corpora heavy with exact-match queries — error codes, CLI commands, ticket IDs, specific tool names. Leave at `1.0` for general prose documentation. |
| `SEARCH_MIN_SCORE` | `0.0` | Drop retrieved chunks below this relevance score before sending context to the LLM. `0.0` keeps everything. Set to `0.3`–`0.4` if you notice irrelevant chunks contaminating answers; leave at `0.0` for small corpora where recall matters more than precision. |
| `RAG_CACHE_TTL_HOURS` | `24` | How long to cache semantically identical answers |
| `RAG_CACHE_THRESHOLD` | `0.95` | Cosine similarity threshold for a query to count as a cache hit |

## Chunking

Controls how documents are split before embedding. See [Ingestion Guide](ingestion.md) for re-ingest instructions.

| Variable | Default | Description |
|----------|---------|-------------|
| `CHUNK_SIZE` | `1500` | Target chunk size in characters. Dense API refs: `800`–`1200`. General docs: `1500`. Long-form prose: `2000`–`2500`. |
| `CHUNK_OVERLAP` | `200` | Overlap between adjacent paragraph-split chunks in characters. |

## OpenSearch Index Names

| Variable | Default | Description |
|----------|---------|-------------|
| `OPENSEARCH_INDEX` | `docbrain-chunks` | Index name for document chunks (vectors + BM25) |
| `OPENSEARCH_EPISODE_INDEX` | `docbrain-episodes` | Index name for episode vectors (used in episodic memory recall) |

Only change these if you run multiple DocBrain instances sharing the same OpenSearch cluster, to avoid index collisions.

## Data Retention

| Variable | Default | Description |
|----------|---------|-------------|
| `EPISODE_RETENTION_DAYS` | `90` | Episode (query history) rows older than this are pruned daily. Set to `0` to disable pruning. |
| `AUDIT_RETENTION_DAYS` | `365` | Audit log rows older than this are pruned daily. Set to `0` to disable pruning. |

## Self-Ingest (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `DOCBRAIN_SELF_INGEST` | `false` | Auto-ingest DocBrain's own docs so it can answer configuration questions about itself |
| `DOCBRAIN_DOCS_PATH` | `./docs` | Path to DocBrain's own documentation directory |

## SSO / OIDC (Enterprise)

| Variable | Default | Description |
|----------|---------|-------------|
| `OIDC_ISSUER_URL` | — | OIDC provider URL (e.g. `https://accounts.google.com`) |
| `OIDC_CLIENT_ID` | — | OAuth client ID |
| `OIDC_CLIENT_SECRET` | — | OAuth client secret |
| `OIDC_REDIRECT_URI` | — | Callback URI (e.g. `http://localhost:3000/auth/oidc/callback`) |
