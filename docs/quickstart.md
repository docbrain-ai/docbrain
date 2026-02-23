# Quickstart

Get DocBrain running in under 5 minutes.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/) with Docker Compose V2
- **For local mode (default):** [Ollama](https://ollama.ai) installed and running — no API keys needed
- **For cloud mode:** An API key from Anthropic or OpenAI

## Option 1: 100% Local with Ollama (Default — No API Keys)

The fastest path. Runs entirely on your hardware — no API keys, no data leaves your machine.

```bash
# Install Ollama (https://ollama.ai) and pull the models
ollama pull llama3.1
ollama pull nomic-embed-text

# Clone and start
git clone https://github.com/docbrain-ai/docbrain.git
cd docbrain
cp .env.example .env    # defaults are already set for Ollama
docker compose up -d
```

> **RAM requirements:** Ollama needs ~8GB for llama3.1 (8B). If your machine has <16GB RAM, use a cloud provider instead (Option 3).

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

Edit `.env` — uncomment your preferred provider section (see comments in the file) and add your API keys. Then:

```bash
docker compose up -d
```

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

Edit `.env` to point at your actual documentation:

**Confluence:**
```env
SOURCE_TYPE=confluence
CONFLUENCE_BASE_URL=https://yourcompany.atlassian.net/wiki
CONFLUENCE_USER_EMAIL=you@yourcompany.com
CONFLUENCE_API_TOKEN=your-api-token
CONFLUENCE_SPACE_KEYS=ENG,DOCS
```

**GitHub:**
```env
SOURCE_TYPE=github
GITHUB_REPO_URL=https://github.com/your-org/your-docs
GITHUB_TOKEN=ghp_...    # only for private repos
```

**Local files:**
```env
SOURCE_TYPE=local
LOCAL_DOCS_PATH=/data/docs
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
