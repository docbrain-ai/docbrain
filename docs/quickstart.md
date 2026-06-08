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

Create `config/local.yaml` (gitignored) and add your source credentials. This
file is never committed — it's the right place for tokens and resource lists.

All ingestion sources live under the top-level `sources:` block. A sub-source
is enabled when its block is present in YAML. Resource lists (repos, projects,
channels, teams) must always be non-empty.

**Confluence:**
```yaml
# config/local.yaml
confluence:
  base_url: https://yourcompany.atlassian.net/wiki
  user_email: you@yourcompany.com
  api_token: your-api-token
  space_keys: ENG,DOCS
```

**GitHub code (clone markdown from repos):**
```yaml
# config/local.yaml
sources:
  github:
    token: ${GITHUB_TOKEN}
    code:
      repos:
        - your-org/your-docs            # default branch
        - your-org/runbooks:develop     # pinned branch
```

**GitHub Pull Requests:**
```yaml
# config/local.yaml
sources:
  github:
    token: ${GITHUB_TOKEN}
    pull_requests:
      repos:
        - your-org/your-repo
        - your-org/infra
      lookback_days: 365
      min_comments: 1
```

**Local files:**
```yaml
# config/local.yaml
sources:
  local:
    path: /data/docs
```

**Multiple providers** (all enabled together):
```yaml
# config/local.yaml
confluence:
  base_url: https://yourcompany.atlassian.net/wiki
  user_email: you@yourcompany.com
  api_token: your-api-token
  space_keys: ENG,DOCS

sources:
  github:
    token: ${GITHUB_TOKEN}
    pull_requests:
      repos:
        - your-org/your-repo
  jira:
    base_url: https://yourcompany.atlassian.net
    user_email: ${JIRA_USER_EMAIL}
    api_token: ${JIRA_API_TOKEN}
    projects:
      - ENG
      - PLAT
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

### Generate a doc

Draft a document grounded in your org's own knowledge (corpus + episodes + live connectors), using a local file as primary material. `generate` returns the markdown — it does not publish.

```bash
docbrain generate "runbook for cert rotation" --source notes.md --type runbook > runbook.md
```

stdout carries only the markdown (pipe-clean, redirect-safe). All diagnostics — resolved doc type, quality score, needs-input questions, skipped sources, violations — go to stderr, so the redirect above captures a clean document. Pass `--out runbook.md` instead of the redirect, or `--source-url <link>` to ground the doc in a Confluence page, Jira issue, Slack thread, or GitHub PR. In CI, the process exits non-zero on error-severity quality violations (override with `--allow-violations`).

See the **[Generate guide](generate.md)** for source links, templates, and CI playbooks.

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
- Is `sources:` in `config/local.yaml` set correctly for at least one provider?
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

## Explore the Web UI

Open **http://localhost:3001** and log in with your API key. Here's what you'll find:

| Page | What It Does |
|---|---|
| **Home** | Dashboard with gap forecast, capture trends, and knowledge health metrics |
| **Ask** | Chat interface — ask questions with streaming answers, source citations, and feedback |
| **Autopilot** | See detected documentation gaps, AI-generated drafts, and review pipeline |
| **Captures** | CI captures, conversation distillation, fragment review queue, and clustering |
| **Governance** | Space ownership, SLA compliance, quality trends, and review workflows |
| **Quality** | Document scores, style rule management, and on-demand linting tool |
| **Events** | Real-time event stream and webhook management |
| **Predictive** | Cascade staleness, seasonal patterns, onboarding gaps, code change analysis |

### Quick wins after first setup

1. **Ask a question** — Go to the Ask page and query your ingested docs. Give thumbs up/down on answers to train the system.
2. **Set up capture** — If you use GitHub PRs, add `POST /api/v1/ci/analyze` to a GitHub Action. Every merged PR will have its decisions, caveats, and procedures extracted automatically.
3. **Create style rules** — Go to Quality → Style Rules and add your team's terminology preferences. Every new document and draft will be linted automatically.
4. **Set up governance** — Go to Governance → Rules & Owners and assign space owners. Set SLA policies to ensure documentation gaps don't linger.

## Next Steps

- [Ingestion Guide](./ingestion.md) — connect Confluence, GitHub, Slack, Jira, and more
- [Governance](./governance.md) — set up space ownership, SLAs, and breach detection
- [Review Workflows](./reviews.md) — configure multi-stage approval pipelines
- [Configuration Reference](./configuration.md) — all environment variables and options
- [Provider Setup](./providers.md) — switch LLM or embedding providers
- [Kubernetes Deployment](./kubernetes.md) — deploy to production
- [API Reference](./api-reference.md) — build integrations
- [Architecture Overview](./architecture.md) — understand how it works
