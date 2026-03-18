# Quickstart

Get DocBrain running in under 5 minutes.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) with Docker Compose V2
- **For local mode (default):** [Ollama](https://ollama.ai) installed and running — no API keys needed
- **For cloud mode:** An API key from Anthropic or OpenAI

## Option 1: 100% Local with Ollama (No API Keys)

The fastest path. Runs entirely on your hardware — no API keys, no data leaves your machine.

```bash
# Install Ollama (https://ollama.ai) and pull the models
ollama pull command-r:35b       # RAG-optimized model with strong instruction following
ollama pull qwen2.5:7b          # fast model for intent classification
ollama pull nomic-embed-text    # embeddings

# Clone and start
git clone https://github.com/docbrain-ai/docbrain.git
cd docbrain
./scripts/setup.sh      # choose option 3 (Ollama) when prompted
```

Or configure manually:

```bash
cp .env.example .env
```

Then edit `.env` and set:
```env
LLM_PROVIDER=ollama
OLLAMA_BASE_URL=http://host.docker.internal:11434
LLM_MODEL_ID=command-r:35b
FAST_MODEL_ID=qwen2.5:7b
EMBED_PROVIDER=ollama
EMBED_MODEL_ID=nomic-embed-text
```

```bash
docker compose up -d
```

> **RAM requirements:** `command-r:35b` needs ~24GB RAM. If your machine has <24GB, use a smaller model such as `llama3.1:8b` for testing, or switch to a cloud provider (Option 3) for production-quality answers. **Do not use 7B-8B models for answer generation in production** — they may fabricate answers. Only models with strong instruction-following should be used for RAG. See [Provider Setup](./providers.md#model-selection--critical-for-answer-quality) for details.

## Option 2: Interactive Setup (Recommended for first-timers)

The setup wizard walks you through everything — provider selection, credentials, and document source configuration:

```bash
git clone https://github.com/docbrain-ai/docbrain.git
cd docbrain
./scripts/setup.sh
```

## Option 3: Cloud Providers (Anthropic / OpenAI)

```bash
git clone https://github.com/docbrain-ai/docbrain.git
cd docbrain
cp .env.example .env
```

Edit `.env` — add your LLM API key (infrastructure secret). Then:

```bash
docker compose up -d
```

For document sources (Confluence, GitHub, Slack, Jira), create `config/local.yaml` (gitignored) with your source credentials. See [Configuration Reference](./configuration.md) for the config-first architecture.

## Verify Installation

```bash
# Check all services are healthy (you should see 5 services: postgres, opensearch, redis, server, web)
docker compose ps

# Get your admin API key (generated on first boot)
docker compose exec server cat /app/admin-bootstrap-key.txt
```

Open http://localhost:3001, go to **Settings**, paste your API key, and start asking questions.

## Ingest Your Documents

DocBrain needs documents to answer questions. **Ingestion** is the process of fetching your docs, converting them to searchable chunks, and indexing them.

### Quick start: Use the included sample docs

The sample docs are already mounted. Just run:

```bash
docker compose exec server docbrain-ingest
```

This takes ~30 seconds. Once done, you can ask questions about the sample documentation.

### Connect your real docs

Create `config/local.yaml` (gitignored) and add your source credentials. This file is never committed — it's the right place for tokens and source settings.

**Confluence:**
```yaml
# config/local.yaml
ingest:
  ingest_sources: confluence

confluence:
  base_url: https://yourcompany.atlassian.net/wiki
  user_email: you@yourcompany.com
  api_token: your-api-token
  space_keys: ENG,DOCS
```

**GitHub repository:**
```yaml
# config/local.yaml
ingest:
  ingest_sources: github

github:
  repo_url: https://github.com/your-org/your-docs
  token: ghp_...    # only for private repos
```

**GitHub Pull Requests:**
```yaml
# config/local.yaml
ingest:
  ingest_sources: github_pr

github_pr:
  token: ghp_...
  repo: your-org/your-repo
  lookback_days: 365
```

**Local files** (no secrets needed — just set in `.env`):
```env
# .env
LOCAL_DOCS_PATH=/data/docs
```

Multiple sources can be combined:
```yaml
# config/local.yaml
ingest:
  ingest_sources: confluence,github_pr,jira
```

Then restart and re-ingest:

```bash
docker compose restart server
docker compose exec server docbrain-ingest
```

For the full walkthrough (API token creation, space key finding, troubleshooting): [Ingestion Guide](./ingestion.md)

## Install the CLI (Standalone)

Install the CLI as a standalone binary — no Docker required:

```bash
# Homebrew (macOS/Linux)
brew install docbrain-ai/tap/docbrain

# npm
npm install -g docbrain

# Shell script (auto-detects OS/arch)
curl -sSL https://raw.githubusercontent.com/docbrain-ai/docbrain/main/scripts/install.sh | sh

# Or download the binary directly (example: macOS Apple Silicon)
curl -sL https://github.com/docbrain-ai/docbrain/releases/latest/download/docbrain-darwin-arm64 -o docbrain
chmod +x docbrain && sudo mv docbrain /usr/local/bin/
```

> All binaries on the [Releases page](https://github.com/docbrain-ai/docbrain/releases/latest):
> `docbrain-darwin-arm64` | `docbrain-darwin-amd64` | `docbrain-linux-amd64`

Configure it to point at your DocBrain server:

```bash
export DOCBRAIN_API_KEY="db_sk_..."
export DOCBRAIN_SERVER_URL="http://localhost:3000"
```

## Use the CLI

The standalone CLI and the Docker-bundled CLI support the same commands:

```bash
# Standalone (after install above)
docbrain ask "How do I deploy to production?"

# Or via Docker (no install needed)
docker compose exec server docbrain-cli ask "How do I deploy to production?"

# View freshness report
docbrain freshness

# Incident mode (prioritizes runbooks)
docbrain incident "API latency spike"

# View analytics
docbrain analytics

# Give feedback on the last answer
docbrain thumbsup
docbrain thumbsdown
```

## Troubleshooting

### Services won't start

```bash
# Check which services are failing
docker compose ps

# View logs for the failing service
docker compose logs server
docker compose logs opensearch
```

**Common causes:**
- **OpenSearch crashes:** Not enough memory. OpenSearch needs ~1GB. Check `docker stats`.
- **Server exits immediately:** Usually a missing or malformed `.env`. Re-copy from `.env.example`.

### "Connection refused" when asking questions

The server takes 10-20 seconds to start. Wait and retry:

```bash
# Check if the server is ready
curl http://localhost:3000/api/v1/config
```

If using Ollama, make sure it's running: `ollama list` should show your models.

### Empty answers or "no relevant documents found"

You need to run ingestion first:

```bash
docker compose exec server docbrain-ingest
```

If ingestion completed but answers are still empty, check:
- Are your documents in a format DocBrain supports? (`.md`, `.txt`, or Confluence/GitHub)
- Is `SOURCE_TYPE` in `.env` set correctly?
- For Confluence: are the space keys correct? Check the URL of your Confluence space.

### Ingestion fails with "401 Unauthorized" (Confluence)

- Double-check `CONFLUENCE_USER_EMAIL` matches the Atlassian account that created the API token
- Regenerate the API token at [https://id.atlassian.com/manage-profile/security/api-tokens](https://id.atlassian.com/manage-profile/security/api-tokens)
- Make sure the URL includes `/wiki`: `https://yourco.atlassian.net/wiki`

### Ollama errors

- **"model not found":** Run `ollama pull llama3.1` and `ollama pull nomic-embed-text`
- **"connection refused":** Make sure Ollama is running (`ollama serve`)
- **On Linux in Docker:** Change `OLLAMA_BASE_URL` from `host.docker.internal` to your machine's local IP
- **Most answers say "Not covered":** Small models (8B) can't follow grounding instructions reliably. Use `command-r:35b` (recommended) or `qwen2.5:32b` for production quality. Set `FAST_MODEL_ID=qwen2.5:7b` to keep intent classification fast.
- **Answers look plausible but are fabricated:** The model is ignoring retrieved documents and generating from training data. This happens with models that have weak instruction-following (including some large models). Switch to `command-r:35b` which is purpose-built for RAG and stays grounded in provided context.
- **"operation timed out":** Large models (32B+) can take 2-3 minutes per answer. Set `OLLAMA_TIMEOUT_SECS=300` in `.env`.

### Resetting everything

If you want to start fresh:

```bash
docker compose down -v    # stops services and deletes all data
docker compose up -d      # start fresh
docker compose exec server docbrain-ingest   # re-ingest
```

## Next Steps

- [Ingestion Guide](./ingestion.md) — connect Confluence, GitHub, or local files
- [Configuration Reference](./configuration.md) — all environment variables
- [Provider Setup](./providers.md) — switch LLM or embedding providers
- [Kubernetes Deployment](./kubernetes.md) — deploy to production
- [API Reference](./api-reference.md) — build integrations
- [Architecture Overview](./architecture.md) — understand how it works
