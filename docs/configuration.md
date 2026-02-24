# Configuration Reference

All configuration is via environment variables, set in `.env` for Docker Compose or via ConfigMap/Secret for Kubernetes.

## Infrastructure

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | — | PostgreSQL connection string |
| `OPENSEARCH_URL` | `http://localhost:9200` | OpenSearch endpoint |
| `REDIS_URL` | `redis://localhost:6379` | Redis connection string |
| `SERVER_PORT` | `3000` | API server listen port |
| `SERVER_BIND` | `0.0.0.0` | API server bind address |

## LLM Provider

| Variable | Default | Description |
|----------|---------|-------------|
| `LLM_PROVIDER` | `bedrock` | Provider: `bedrock`, `anthropic`, `openai`, `ollama` |
| `LLM_MODEL_ID` | varies | Model identifier (provider-specific) |
| `LLM_THINKING_BUDGET` | `10000` | Max thinking tokens for extended thinking models |
| `ANTHROPIC_API_KEY` | — | API key (if `LLM_PROVIDER=anthropic`) |
| `OPENAI_API_KEY` | — | API key (if `LLM_PROVIDER=openai`) |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama server URL |
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
| `CONFLUENCE_USER_EMAIL` | — | Confluence authentication email |
| `CONFLUENCE_API_TOKEN` | — | Confluence API token |
| `CONFLUENCE_SPACE_KEYS` | — | Comma-separated space keys to ingest |
| `GITHUB_REPO_URL` | — | Repository URL to clone and ingest |
| `GITHUB_TOKEN` | — | GitHub personal access token (optional for public repos) |
| `GITHUB_BRANCH` | `main` | Branch to ingest from |

## Image Extraction

| Variable | Default | Description |
|----------|---------|-------------|
| `IMAGE_EXTRACTION_ENABLED` | `true` | Extract and describe images from Confluence pages using vision LLM. Set to `false` to disable. |
| `HAIKU_MODEL_ID` | — | Model used for image descriptions (cheaper/faster). Falls back to `LLM_MODEL_ID` if not set. |

Image extraction requires a vision-capable LLM. Supported providers: **Bedrock**, **Anthropic**, **OpenAI**, and **Ollama** (with vision models like `llava`, `llama3.2-vision`, `moondream`). Text-only models (e.g. `llama3.1`) are auto-detected and images are skipped gracefully — no failures, no errors.

## Web UI / CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3001` | Comma-separated origins allowed to call the API. Only needed if the web UI is served from a non-default origin (e.g. `http://10.0.0.5:3001`, `https://docbrain.internal`) |

> **Note:** The default works out of the box for Docker Compose. You only need this if you access the web UI via a different hostname or port — for example, `http://127.0.0.1:3001` is a different origin than `http://localhost:3001`.

## Slack Integration (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `SLACK_BOT_TOKEN` | — | Slack bot OAuth token (`xoxb-...`) |
| `SLACK_SIGNING_SECRET` | — | Slack app signing secret |

## Documentation Autopilot (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `AUTOPILOT_ENABLED` | `false` | Enable the Documentation Autopilot (gap detection + draft generation) |

When enabled, Autopilot runs daily gap analysis on unanswered queries and exposes management endpoints at `/api/v1/autopilot/*`. See the [API Reference](api-reference.md) for endpoint details and the [Architecture doc](architecture.md) for how the pipeline works.

## Notifications (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `NOTIFICATION_INTERVAL_HOURS` | `24` | How often to check for stale docs |
| `NOTIFICATION_SPACE_FILTER` | — | Limit notifications to a specific space |
