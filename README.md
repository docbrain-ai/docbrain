<p align="center">
  <img src="assets/banner.png" alt="DocBrain" width="600" />
</p>

<p align="center">
  <strong>Source-available documentation intelligence engine.</strong><br/>
  Answers questions. Learns from usage. Identifies gaps. Drafts what's missing.
</p>

<p align="center">
  <a href="https://github.com/docbrain-ai/docbrain/stargazers"><img src="https://img.shields.io/github/stars/docbrain-ai/docbrain?style=social" alt="Stars" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-BSL%201.1-blue" alt="License" /></a>
  <a href="https://github.com/docbrain-ai/docbrain/releases/latest"><img src="https://img.shields.io/github/v/release/docbrain-ai/docbrain" alt="Release" /></a>
  <a href="https://github.com/docbrain-ai/docbrain/releases/latest"><img src="https://img.shields.io/badge/built_with-Rust-orange" alt="Rust" /></a>
</p>

<p align="center">
  <a href="#quickstart">Quickstart</a> &bull;
  <a href="#how-it-works">How It Works</a> &bull;
  <a href="#documentation-autopilot">Autopilot</a> &bull;
  <a href="docs/architecture.md">Architecture</a> &bull;
  <a href="docs/api-reference.md">API Reference</a> &bull;
  <a href="docs/kubernetes.md">Deploy</a>
</p>

---

## Why DocBrain Exists

Documentation has the same fundamental problem everywhere: it degrades the moment it's published. Docs go stale, knowledge gaps grow invisibly, and the only signal that something is missing is a frustrated engineer asking in Slack.

Existing tools try to fix this with better search. DocBrain takes a different approach: it treats documentation as a **living system** — one that should monitor its own health, surface its own gaps, and propose its own improvements.

```mermaid
graph LR
    A["Team asks questions"] --> B["DocBrain answers<br/>(with memory + freshness)"]
    B --> C["Unanswered? Low confidence?"]
    C --> D["Autopilot clusters gaps"]
    D --> E["Drafts missing docs"]
    E --> F["Team reviews + publishes"]
    F --> A
    style D fill:#2563eb,color:#fff
    style E fill:#2563eb,color:#fff
```

The more your team uses DocBrain, the better your documentation gets. That feedback loop is the core idea.

---

## Quickstart

```bash
git clone https://github.com/docbrain-ai/docbrain.git && cd docbrain
cp .env.example .env
# Edit .env — pick your LLM provider (see options below)
docker compose up -d
```

```bash
# Get your admin API key (copy the output)
docker compose exec server cat /app/admin-bootstrap-key.txt

# Ingest the included sample docs
docker compose exec server docbrain-ingest

# Ask a question (replace <key> with the key from above)
docker compose exec -e DOCBRAIN_API_KEY=<key> server docbrain-cli ask "How do I deploy to production?"
```

Open the Web UI at **http://localhost:3001**.

### Choose Your LLM Provider

<details open>
<summary><strong>AWS Bedrock (recommended for teams already on AWS)</strong></summary>

Uses your existing AWS credentials. No separate API keys needed.

```env
LLM_PROVIDER=bedrock
LLM_MODEL_ID=us.anthropic.claude-sonnet-4-5-20250929-v1:0
EMBED_PROVIDER=bedrock
EMBED_MODEL_ID=cohere.embed-v4:0
AWS_REGION=us-east-1
```

Ensure the models are enabled in your [Bedrock Model Access](https://console.aws.amazon.com/bedrock/home#/modelaccess) console. Credentials are picked up from environment variables, `~/.aws/credentials`, or instance profiles.

</details>

<details>
<summary><strong>Anthropic + OpenAI (best quality)</strong></summary>

```env
LLM_PROVIDER=anthropic
ANTHROPIC_API_KEY=sk-ant-...
LLM_MODEL_ID=claude-sonnet-4-5-20250929
EMBED_PROVIDER=openai
OPENAI_API_KEY=sk-...
EMBED_MODEL_ID=text-embedding-3-small
```

</details>

<details>
<summary><strong>OpenAI only (single API key)</strong></summary>

```env
LLM_PROVIDER=openai
OPENAI_API_KEY=sk-...
LLM_MODEL_ID=gpt-4o
EMBED_PROVIDER=openai
EMBED_MODEL_ID=text-embedding-3-small
```

</details>

<details>
<summary><strong>Ollama (100% local, no data leaves your machine)</strong></summary>

```bash
ollama pull llama3.1 && ollama pull nomic-embed-text
```

```env
LLM_PROVIDER=ollama
OLLAMA_BASE_URL=http://host.docker.internal:11434
LLM_MODEL_ID=llama3.1
EMBED_PROVIDER=ollama
EMBED_MODEL_ID=nomic-embed-text
```

</details>

<details>
<summary><strong>Interactive setup wizard</strong></summary>

```bash
git clone https://github.com/docbrain-ai/docbrain.git && cd docbrain
./scripts/setup.sh
```

Walks you through provider selection, API key configuration, and document source setup.

</details>

See [Provider Setup](docs/providers.md) for full configuration details.

---

## How It Works

DocBrain is a RAG pipeline with three layers that most implementations skip: **memory**, **freshness awareness**, and **autonomous gap detection**.

```mermaid
graph TB
    Q["Question"] --> IC["Intent Classification<br/><i>factual · procedural · troubleshooting · comparative</i>"]
    IC --> QR["Query Rewriting<br/><i>using conversation context</i>"]
    QR --> HS["Hybrid Search<br/><i>k-NN vectors + BM25 keywords</i>"]
    QR --> ML["Memory Lookup<br/><i>episodic · semantic · procedural</i>"]
    HS --> CA["Context Assembly"]
    ML --> CA
    CA --> FS["Freshness Check<br/><i>flag stale sources</i>"]
    FS --> LLM["LLM Generation<br/><i>streaming, with citations</i>"]
    LLM --> R["Answer + Sources + Confidence"]
    R --> EP["Episode Storage"]
    EP -. "feedback loop" .-> AP["Autopilot<br/><i>gap detection · draft generation</i>"]

    style AP fill:#2563eb,color:#fff
    style FS fill:#059669,color:#fff
    style ML fill:#7c3aed,color:#fff
```

### Memory System

Most Q&A tools are stateless — every question starts from zero. DocBrain maintains four tiers of memory:

| Tier | Purpose | Example |
|------|---------|---------|
| **Working** | Conversation context within a session | "by 'the service' I mean auth-service" |
| **Episodic** | Past Q&A across all users, with feedback | "this was asked before — validated answer exists" |
| **Semantic** | Entity graph — services, teams, dependencies | "auth-service depends on Redis, owned by Platform" |
| **Procedural** | Rules learned from feedback patterns | "for deploy questions, always include the canary step" |

Working memory is session-scoped (Redis). The other three are permanent (PostgreSQL + OpenSearch) and compound over time.

### Freshness Scoring

Every indexed document receives a freshness score from 0 to 100, recalculated on a configurable schedule. Stale sources are flagged in answers so users know when to treat information with caution.

| Signal | Weight | What It Measures |
|--------|--------|-----------------|
| Time Decay | 30% | Time since last edit |
| Engagement | 20% | Query frequency, view count, feedback ratio |
| Content Currency | 20% | LLM analysis of temporal references ("as of Q1 2024") |
| Link Health | 15% | Broken or redirected links within the document |
| Contradiction | 15% | Cross-document consistency (does this conflict with other docs?) |

Documents scoring below 40 are flagged as outdated. When combined with Autopilot data, this surfaces docs that are both stale *and* frequently asked about — the highest-impact content to fix.

### Intent-Adaptive Responses

DocBrain classifies each query and adapts the response format:

| Intent | Response Format |
|--------|----------------|
| Factual | Direct answer with source citation |
| Procedural | Numbered step-by-step instructions |
| Troubleshooting | Diagnostic tree with ranked causes |
| Comparative | Structured comparison table |
| Incident | Runbooks and playbooks surfaced first |

### Multi-Team / Space-Aware Search

When connected to Confluence with multiple spaces (or multiple doc sources), DocBrain tracks which space each chunk belongs to. This matters when different teams have their own runbooks, deployment guides, or incident procedures.

**How it works:**
- Every indexed chunk carries its Confluence space key (or source type for local/GitHub docs)
- Pass `"space": "PLATFORM"` in your `/api/v1/ask` request to boost results from that team's docs
- Results from other spaces still appear — they just rank lower (soft boost, not hard filter)
- The LLM sees `[Chunk 1] "Deploy Guide" | Space: PLATFORM` in its context, so it can distinguish between team-specific answers
- Procedural rules can define `boost_spaces` to automatically prefer certain spaces for certain query patterns

**In the response**, each source includes:
```json
{
  "title": "Production Deployment Guide",
  "source_url": "https://confluence.example.com/...",
  "space": "PLATFORM",
  "freshness_score": 82.0,
  "freshness_status": "fresh",
  "score": 14.2
}
```

Sources flagged as `stale` or `outdated` are visually marked so users know to verify the information.

### Team-Scoped API Keys

For organizations with multiple teams sharing one DocBrain instance, API keys can be scoped to specific spaces. This provides hard data isolation without deploying separate instances.

```bash
# Create a key that only sees PLATFORM and SRE docs
curl -X POST http://localhost:3000/api/v1/admin/keys \
  -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"name": "Platform Team Key", "role": "editor", "allowed_spaces": ["PLATFORM", "SRE"]}'
```

**How it works:**
- `allowed_spaces` on an API key = hard filter. Queries using this key can only see results from those spaces.
- The `"space"` parameter in `/api/v1/ask` = soft boost (ranks matching space higher, but doesn't hide other results).
- Both can be combined: a Platform team key scoped to `["PLATFORM", "SRE"]` with a request-level `"space": "PLATFORM"` gives strongest preference to Platform docs while still allowing SRE cross-references.
- Keys without `allowed_spaces` see everything (default behavior, backward compatible).

### Knowledge Base Health Dashboard

A single endpoint that tells you the health of your entire documentation corpus:

```bash
curl http://localhost:3000/api/v1/health/report \
  -H "Authorization: Bearer $API_KEY"
```

Returns:
```json
{
  "total_documents": 342,
  "total_spaces": 8,
  "overall_health_score": 67.3,
  "freshness_distribution": { "fresh": 120, "review": 89, "stale": 72, "outdated": 41, "archive": 20 },
  "spaces": [
    { "space": "PLATFORM", "doc_count": 86, "avg_freshness": 71.2, "stale_count": 12, "queries_last_7d": 234 },
    { "space": "SRE", "doc_count": 54, "avg_freshness": 58.1, "stale_count": 18, "queries_last_7d": 187 }
  ],
  "top_stale_cited_docs": [
    { "title": "Deploy Guide", "freshness_score": 23, "citations_last_7d": 47, "contradiction_score": 45 }
  ],
  "coverage_gaps": 15
}
```

The `top_stale_cited_docs` list surfaces the documents that are simultaneously stale AND frequently served to users -- the highest-impact docs to fix first.

### AI-Curated Onboarding

New team member joining? Get an AI-curated reading list tailored to their first week:

```bash
curl http://localhost:3000/api/v1/onboard/ENG \
  -H "Authorization: Bearer $API_KEY"
```

```json
{
  "space": "ENG",
  "total_docs": 694,
  "reading_list": [
    {
      "title": "SaaS UI Engineer Onboarding Guide",
      "freshness_score": 85,
      "freshness_status": "fresh",
      "popularity": 12,
      "reason": "Direct onboarding guide covering role-specific processes and expectations."
    },
    {
      "title": "Team Overview & What We Own",
      "freshness_score": 92,
      "freshness_status": "fresh",
      "popularity": 8,
      "reason": "Gives new hires context on team mission and responsibilities in week 1."
    }
  ]
}
```

This is **not** a simple SQL sort. Under the hood:

1. **Semantic search** -- 5 onboarding-intent queries ("new hire getting started", "team overview what we own", "how to set up dev environment", etc.) find docs that match what a week-1 person actually needs
2. **LLM curation** -- an LLM selects the best 8-12 docs from candidates, explicitly filtering out meeting notes, sprint pages, deep technical specs (HLDs/LLDs), and infrastructure runbooks
3. **Week-1 persona** -- the LLM asks "would a new hire read this in their first 5 days?" and excludes anything that belongs in month 2+

The result: onboarding guides, team overviews, dev setup docs, and process norms -- not autoscaling configs and auth protocol designs.

### Contradiction Detection

DocBrain's freshness scorer runs a 4th pass that compares each document against others in the same space, detecting conflicting instructions, inconsistent config values, and contradictory ownership claims.

When contradictions are detected (score < 70), they surface in API responses:

```json
{
  "sources": [{
    "title": "Deploy Guide v2",
    "space": "PLATFORM",
    "freshness_score": 45,
    "freshness_status": "stale",
    "contradiction_score": 32
  }]
}
```

A `contradiction_score` of 32 means this doc significantly contradicts other docs in the same space. Combined with the health dashboard, you can prioritize which docs need reconciliation.

### Owner-Aware Stale Notifications

DocBrain sends targeted Slack DMs to document owners when their docs go stale:

> :warning: **Stale Documentation Alert**
>
> 1. [Deploy Guide](https://...) -- freshness 23/100 . **cited 47 times this week**
> 2. [API Rate Limits](https://...) -- freshness 31/100 . **cited 12 times this week**

The citation count makes the notification actionable -- "this doc is stale AND your team is actively relying on it." High-impact stale docs trigger a `:rotating_light: High Impact` urgency marker.

Configure via:
```env
NOTIFICATION_INTERVAL_HOURS=24
NOTIFICATION_SPACE_FILTER=PLATFORM,SRE  # optional: limit to specific spaces
```

---

## Documentation Autopilot

Autopilot is what turns DocBrain from a consumption tool into a documentation improvement system.

It runs on a daily schedule, analyzing every query from the past 30 days that received negative feedback, a `not_found` resolution, or a confidence score below 0.4. It clusters these queries by semantic similarity, identifies patterns, and creates actionable gap reports.

```mermaid
graph TB
    subgraph "Daily Analysis"
        E["Episodic Memory<br/>(30-day window)"] --> F["Filter: negative feedback,<br/>not_found, low confidence"]
        F --> EM["Embed queries"]
        EM --> CL["Cosine similarity clustering<br/>(threshold: 0.82)"]
        CL --> LB["Label clusters via LLM"]
        LB --> DB["Persist gap clusters<br/>with severity rating"]
    end

    subgraph "On Demand"
        DB --> GEN["Generate draft"]
        GEN --> CTX["Search existing docs<br/>for partial context"]
        CTX --> CLS["Classify content type<br/>(runbook · FAQ · guide · reference)"]
        CLS --> DFT["LLM drafts document"]
        DFT --> REV["Human review"]
        REV --> PUB["Publish + re-ingest"]
    end

    style DB fill:#2563eb,color:#fff
    style DFT fill:#2563eb,color:#fff
```

**What this produces:**

- **Gap clusters** ranked by severity (critical / high / medium / low) based on query volume. A cluster like "Production Deployment Process" with 47 unanswered queries is flagged as critical.
- **Auto-generated drafts** — runbooks, FAQs, troubleshooting guides, or reference docs. DocBrain uses your existing documentation as context, so drafts match your team's domain language.
- **Weekly digests** — a Slack summary of total questions asked, unanswered rate, top gaps, and drafts ready for review.

Enable it with one environment variable:

```env
AUTOPILOT_ENABLED=true
```

Autopilot endpoints: [`GET /api/v1/autopilot/gaps`](docs/api-reference.md), [`POST /api/v1/autopilot/generate/{id}`](docs/api-reference.md), [`GET /api/v1/autopilot/digest`](docs/api-reference.md) — [full API reference](docs/api-reference.md).

---

## Connect Your Documents

DocBrain ingests from three source types. Documents are chunked with heading-aware splitting, embedded, and indexed in OpenSearch. Ingestion runs on a configurable cron schedule for continuous sync.

<details>
<summary><strong>Confluence</strong></summary>

```env
SOURCE_TYPE=confluence
CONFLUENCE_BASE_URL=https://yourcompany.atlassian.net/wiki
CONFLUENCE_USER_EMAIL=you@yourcompany.com
CONFLUENCE_API_TOKEN=your-token
CONFLUENCE_SPACE_KEYS=ENG,DOCS,OPS
```

</details>

<details>
<summary><strong>GitHub Repository</strong></summary>

```env
SOURCE_TYPE=github
GITHUB_REPO_URL=https://github.com/your-org/your-docs
GITHUB_TOKEN=ghp_...    # only needed for private repos
GITHUB_BRANCH=main
```

</details>

<details>
<summary><strong>Local Markdown Files</strong></summary>

```env
SOURCE_TYPE=local
LOCAL_DOCS_PATH=/data/docs
```

</details>

Full ingestion guide with troubleshooting: [docs/ingestion.md](docs/ingestion.md)

---

## Integrations

### MCP (Model Context Protocol)

Query your documentation — and discover gaps — from Claude Code, Cursor, or any MCP-compatible editor.

**Claude Code:**

```bash
claude mcp add docbrain -- npx -y docbrain-mcp@latest
```

That's it. Set your API key and server URL as environment variables, or pass them inline:

```bash
claude mcp add docbrain \
  -e DOCBRAIN_API_KEY=db_sk_... \
  -e DOCBRAIN_SERVER_URL=http://localhost:3000 \
  -- npx -y docbrain-mcp@latest
```

**Cursor** (`.cursor/mcp.json`):

```json
{
  "mcpServers": {
    "docbrain": {
      "command": "npx",
      "args": ["-y", "docbrain-mcp@latest"],
      "env": {
        "DOCBRAIN_API_KEY": "db_sk_...",
        "DOCBRAIN_SERVER_URL": "http://localhost:3000"
      }
    }
  }
}
```

<details>
<summary><strong>Alternative: use the binary directly (no npx)</strong></summary>

If you installed via Homebrew or downloaded the binary:

```json
{
  "mcpServers": {
    "docbrain": {
      "command": "docbrain-mcp",
      "env": {
        "DOCBRAIN_API_KEY": "db_sk_...",
        "DOCBRAIN_SERVER_URL": "http://localhost:3000"
      }
    }
  }
}
```

</details>

<details>
<summary><strong>Alternative: Docker</strong></summary>

If you don't have Node.js and prefer Docker:

```json
{
  "mcpServers": {
    "docbrain": {
      "command": "docker",
      "args": [
        "run", "--rm", "-i", "--network", "host",
        "-e", "DOCBRAIN_SERVER_URL=http://localhost:3000",
        "-e", "DOCBRAIN_API_KEY=db_sk_...",
        "ghcr.io/docbrain-ai/docbrain:latest",
        "docbrain-mcp"
      ]
    }
  }
}
```

</details>

**Available MCP tools:** `docbrain_ask`, `docbrain_incident`, `docbrain_freshness`, `docbrain_autopilot_gaps`, `docbrain_autopilot_generate`, `docbrain_autopilot_summary`

### Slack

DocBrain can serve as a Slack bot for team-wide Q&A and receive weekly Autopilot digests. Set `SLACK_BOT_TOKEN` and `SLACK_SIGNING_SECRET` — see [Configuration](docs/configuration.md).

### CLI

```bash
brew install docbrain-ai/tap/docbrain
# or: npm install -g docbrain
# or: curl -sSL https://raw.githubusercontent.com/docbrain-ai/docbrain/main/scripts/install.sh | sh
```

```bash
export DOCBRAIN_API_KEY="db_sk_..."
export DOCBRAIN_SERVER_URL="http://localhost:3000"

docbrain ask "How do I configure mTLS between services?"
docbrain freshness --space PLATFORM
docbrain incident "Redis connection timeouts in auth-service"
```

---

## Deploy

### Docker Compose (default)

```bash
docker compose up -d
```

Starts the API server, web UI, PostgreSQL, OpenSearch, and Redis. Schema migrations run automatically on boot.

### Kubernetes (Helm)

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.anthropicApiKey=$ANTHROPIC_API_KEY \
  --set autopilot.enabled=true
```

Supports external PostgreSQL, OpenSearch, and Redis. Includes HPA, ingress, and TLS configuration. See [Kubernetes docs](docs/kubernetes.md).

---

## Architecture

```mermaid
graph TB
    subgraph "Clients"
        WEB["Web UI<br/>(Next.js)"]
        CLI["CLI"]
        MCP["MCP Server"]
        SLACK["Slack Bot"]
    end

    subgraph "DocBrain Server (Rust / Axum)"
        API["REST API + SSE"]
        AUTH["Auth + RBAC"]
        RAG["RAG Pipeline"]
        AUTO["Autopilot Engine"]
        FRESH["Freshness Scorer"]
    end

    subgraph "Storage"
        PG["PostgreSQL<br/><i>memory · episodes · entities<br/>rules · gap clusters · drafts</i>"]
        OS["OpenSearch<br/><i>vector index (k-NN)<br/>keyword index (BM25)</i>"]
        RD["Redis<br/><i>sessions · cache<br/>rate limits</i>"]
    end

    subgraph "LLM Providers"
        OL["Ollama<br/>(local)"]
        AN["Anthropic"]
        OA["OpenAI"]
        BR["AWS Bedrock"]
    end

    WEB & CLI & MCP & SLACK --> API
    API --> AUTH --> RAG
    API --> AUTO
    API --> FRESH
    RAG --> PG & OS & RD
    RAG --> OL & AN & OA & BR
    AUTO --> PG & OS
    AUTO --> AN & OA & BR
    FRESH --> PG
```

| Component | Technology | Role |
|-----------|-----------|------|
| API Server | Rust, Axum, Tower | HTTP/SSE, auth, rate limiting, routing |
| RAG Pipeline | Custom | Intent classification, hybrid search, memory enrichment, generation |
| Autopilot | Custom | Gap analysis, semantic clustering, draft generation, digests |
| Freshness | Custom | 5-signal scoring, contradiction detection, staleness alerts |
| Storage | PostgreSQL 17, OpenSearch 2.19, Redis 7 | Metadata, vectors, cache |
| Ingestion | Custom | Confluence, GitHub, local file connectors with heading-aware chunking |

Full architecture documentation: [docs/architecture.md](docs/architecture.md)

---

## Documentation

| | |
|---|---|
| [Quickstart](docs/quickstart.md) | Running locally or in the cloud in 5 minutes |
| [Ingestion Guide](docs/ingestion.md) | Connecting Confluence, GitHub, or local files |
| [Configuration](docs/configuration.md) | All environment variables and options |
| [Provider Setup](docs/providers.md) | LLM and embedding provider configuration |
| [Architecture](docs/architecture.md) | System design, data flow, memory, freshness, and Autopilot |
| [API Reference](docs/api-reference.md) | REST API with Autopilot endpoints |
| [Kubernetes](docs/kubernetes.md) | Helm chart deployment and scaling |

---

## Contributing

We welcome bug reports, feature requests, and documentation improvements via [GitHub Issues](https://github.com/docbrain-ai/docbrain/issues).

Source code releases at **5,000 GitHub stars** or **January 1, 2028**, whichever comes first.

## License

[Business Source License 1.1](LICENSE) — free to use, deploy, and modify. Cannot be offered as a competing hosted service. Converts to Apache 2.0 on the date above.
