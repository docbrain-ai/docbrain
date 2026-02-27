# Configuration Reference

## How Configuration Works

DocBrain uses a **layered YAML + environment variable** system. Understanding this prevents confusion about why a value isn't taking effect.

### Loading Order (later = higher priority)

```
config/default.yaml         ← committed to repo, all non-sensitive defaults
config/{APP_ENV}.yaml       ← environment-specific overrides (development | production)
config/local.yaml           ← gitignored personal overrides (optional, dev only)
Environment variables / .env ← always win — use for secrets and deployment values
```

Set `APP_ENV=production` for the production profile (this is the default in the Docker image). The server defaults to `APP_ENV=development` when running locally without Docker.

### What Goes Where

| Type | Where to put it |
|---|---|
| Secrets (API keys, DB passwords, tokens) | `.env` or environment variables |
| Deployment-specific values (URLs, ports, CORS origins) | `.env` or environment variables |
| Tuning (thresholds, intervals, cache TTLs) | `config/local.yaml` or env vars |
| Team-wide defaults you want committed | `config/default.yaml` (no secrets!) |

### YAML Config Structure

The YAML files mirror the environment variable names but are grouped by section:

```yaml
# config/local.yaml — gitignored, safe for personal dev overrides
autopilot:
  enabled: true
  cluster_threshold: 0.78    # looser clustering for testing

rag:
  cache_ttl_hours: 1         # short cache in dev

slack:
  notification_interval_hours: 1
```

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

| Variable | Default | Description |
|----------|---------|-------------|
| `SOURCE_TYPE` | `confluence` | Source: `local`, `confluence`, `github` |
| `LOCAL_DOCS_PATH` | — | Directory path for local file ingestion |
| `CONFLUENCE_BASE_URL` | — | Atlassian instance URL (must include `/wiki`, e.g. `https://yourco.atlassian.net/wiki`) |
| `CONFLUENCE_USER_EMAIL` | — | Confluence authentication email (not required for v1 Data Center) |
| `CONFLUENCE_API_TOKEN` | — | API token (Cloud) or Personal Access Token (Data Center) |
| `CONFLUENCE_SPACE_KEYS` | — | Comma-separated space keys to ingest |
| `CONFLUENCE_PAGE_LIMIT` | `0` (unlimited) | Max pages to ingest per space. Set to a positive number to cap results (e.g. `100`). `0` = ingest all pages. |
| `CONFLUENCE_API_VERSION` | `v2` | API version: `v2` for Cloud, `v1` for self-hosted Data Center 7.x+ |
| `CONFLUENCE_TLS_VERIFY` | `true` | Set to `false` to skip TLS certificate verification (for self-signed or internal CA certs) |
| `GITHUB_REPO_URL` | — | Repository URL to clone and ingest |
| `GITHUB_TOKEN` | — | GitHub personal access token (optional for public repos) |
| `GITHUB_BRANCH` | `main` | Branch to ingest from |

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

## RAG Cache

| Variable | Default | Description |
|----------|---------|-------------|
| `RAG_CACHE_TTL_HOURS` | `24` | How long to cache semantically identical answers |
| `RAG_CACHE_THRESHOLD` | `0.95` | Cosine similarity threshold for a query to count as a cache hit |

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
