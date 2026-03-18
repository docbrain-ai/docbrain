<p align="center">
  <img src="assets/banner.png" alt="DocBrain" width="600" />
</p>

<p align="center">
  <strong>Self-improving documentation intelligence for teams.</strong><br/>
  DocBrain ingests knowledge from every tool your team uses, answers questions with source attribution and confidence scoring, and autonomously identifies documentation gaps — turning every unanswered question into a documented solution.
</p>

<p align="center">
  <a href="https://docbrainapi.com"><img src="https://img.shields.io/badge/website-docbrainapi.com-6366f1" alt="Website" /></a>
  <a href="https://github.com/docbrain-ai/docbrain/stargazers"><img src="https://img.shields.io/github/stars/docbrain-ai/docbrain?style=social" alt="Stars" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-BSL%201.1-blue" alt="License" /></a>
  <img src="https://img.shields.io/badge/built_with-Rust-orange" alt="Rust" />
  <a href="https://glama.ai/mcp/servers/docbrain-ai/docbrain"><img src="https://glama.ai/mcp/servers/docbrain-ai/docbrain/badge" alt="MCP" height="20" /></a>
</p>

<p align="center">
  <a href="https://docbrainapi.com"><strong>Website</strong></a> &bull;
  <a href="#quickstart">Quickstart</a> &bull;
  <a href="#key-features">Features</a> &bull;
  <a href="#architecture">Architecture</a> &bull;
  <a href="#documentation">Docs</a> &bull;
  <a href="#community">Community</a> &bull;
  <a href="#contributing">Contributing</a>
</p>

---

> **Project Status:** DocBrain is currently distributed as pre-built Docker images and deployment artifacts (Helm charts, configuration, documentation). Source code is not yet published. CI/CD pipelines, build-from-source instructions, and automated test suites will be added when the source is released. Contributions are currently welcome for documentation, configuration, and bug reports against the published artifacts.

---

## Overview

DocBrain is a RAG-based documentation intelligence platform built in Rust. It connects to 13+ knowledge sources (Confluence, Slack, GitHub, Jira, PagerDuty, Zendesk, Microsoft Teams, and more), provides confidence-scored answers with source attribution, and runs an autonomous **Autopilot** that detects documentation gaps and drafts missing content.

Unlike static search tools, DocBrain maintains a multi-tier memory system that compounds over time — every question, answer, and feedback signal makes the next response better.

### See It In Action

| | |
|---|---|
| [What is DocBrain?](https://youtu.be/S4aSTmevvOQ) — 5-min overview | [Deep Dive Podcast](https://youtu.be/GN4SC6L8YmI) — 20-min deep dive |
| [MCP Preview](https://youtu.be/9mZLoQnGLl8) — 30-sec IDE demo | [Full Proof Demo](https://youtu.be/yqj5BCVOLHw) — Downvote → Gap → Draft |

---

## Key Features

- **13+ Knowledge Sources** — Confluence, Slack, Microsoft Teams, GitHub PRs, GitLab MRs, Jira, PagerDuty, OpsGenie, Zendesk, Intercom, local Markdown, and more
- **Confidence-Scored Answers** — Zero-guess policy: high confidence returns sourced answers, low confidence asks clarifying questions instead of hallucinating
- **Documentation Autopilot** — Autonomously clusters unanswered questions, detects gaps, and drafts missing documentation using your org's existing voice
- **4-Tier Memory System** — Working, episodic, semantic, and procedural memory that compounds with every interaction
- **Document Health Scores** — 5-signal freshness scoring (time decay, engagement, content currency, link health, contradiction detection) with proactive staleness alerts
- **Cross-Document Reference Graph** — Automatically extracts and links references across documents (GitHub PRs, GitLab MRs, Jira tickets, Confluence pages) for richer context during retrieval
- **Real-Time Capture** — `/docbrain capture` in Slack threads, `@docbrain capture` on GitHub PRs and GitLab MRs for instant knowledge indexing
- **Intent-Adaptive Responses** — Classifies queries (find, how-to, troubleshoot, who-owns, status, explain) and adapts response format accordingly
- **Image Intelligence** — Vision-capable LLM extraction of architecture diagrams, flowcharts, and screenshots during ingestion
- **Multi-Team Space Isolation** — Soft boost, per-request filters, and API-key-level hard restrictions for multi-tenant deployments
- **Multiple LLM Providers** — Anthropic, OpenAI, AWS Bedrock, and Ollama (fully local, air-gapped)

---

## Intelligence Layer

DocBrain's intelligence layer goes beyond retrieval with five systems that make it proactive, self-improving, and organizationally aware:

### Knowledge Graph
BFS/DFS traversal over your entity graph surfaces structural knowledge: "What depends on the auth service?", "Who are the experts on Kubernetes?", "What's the blast radius if Redis goes down?" Graph traversal answers questions that no amount of vector similarity can.

**API:** `GET /api/v1/graph/entity/:name`, `GET /api/v1/graph/blast-radius/:entity_id`, `GET /api/v1/graph/experts/:topic`

### Documentation Analytics
Documentation health, quantified. Daily snapshots measure gap resolution rate, knowledge half-life, tribal knowledge %, and ROI in USD. Per-team breakdowns show which teams are investing in knowledge quality and which are accumulating undocumented tribal expertise.

**API:** `GET /api/v1/analytics/velocity`, `GET /api/v1/analytics/velocity/teams`

### Predictive Intelligence
Gaps before they become incidents. DocBrain detects cascade staleness (updating one doc flags its dependents), forecasts seasonal query spikes, identifies onboarding friction for new hires, and flags documentation likely affected by recent code changes.

**API:** `GET /api/v1/predictive/cascade`, `GET /api/v1/predictive/seasonal`, `GET /api/v1/predictive/onboarding`, `POST /api/v1/predictive/code-change`

### Autonomous Document Maintenance
Every documentation system accumulates entropy: contradictions between runbooks, links that rot, version numbers that drift. DocBrain generates targeted fix proposals and presents them for one-click review. Human in the loop, but not human in the workflow.

**API:** `GET /api/v1/maintenance/fixes`, `POST /api/v1/maintenance/fixes/:id/apply`

### Knowledge Stream
Proactive push intelligence. Incident early warnings fire when multiple users cluster on the same topic. Decay risk alerts reach document authors. Expertise gap alerts surface single-point-of-failure knowledge areas before they become incidents.

**API:** `GET /api/v1/stream/events`, `POST /api/v1/stream/context`

See [docs/knowledge-intelligence.md](docs/knowledge-intelligence.md) for full API reference and configuration.

---

## Learning Pipeline (Tier 2, opt-in)

DocBrain can improve its own retrieval quality by fine-tuning its embedding model on your team's feedback. Disabled by default — no infrastructure overhead unless you enable it.

### Progressive Tiers

| Tier | What's Active | Infrastructure Required |
|------|--------------|------------------------|
| **Tier 0** | Fixed pre-trained embeddings | Nothing (default) |
| **Tier 1** | Feedback collection + training data storage | Object storage (S3, GCS, or Azure) |
| **Tier 2** | Full fine-tuning + ONNX hot-swap | Tier 1 + compute (2 vCPU / 8 GB minimum) |

Safety: training data quality guards reject corpora dominated by a single user. Automatic rollback triggers on quality regression.

```bash
# Enable Tier 2:
LEARNING_ENABLED=true
EMBEDDING_PROVIDER=local
TRAINER_URL=http://trainer:8765
```

See [docs/learning.md](docs/learning.md) for full setup and configuration.

---

## Quickstart

### Prerequisites

- Docker and Docker Compose
- An LLM API key (Anthropic, OpenAI, AWS Bedrock) or [Ollama](https://ollama.com) for local inference

### Run

```bash
git clone https://github.com/docbrain-ai/docbrain.git && cd docbrain
./scripts/setup.sh    # interactive wizard — picks provider, sets keys, starts services
```

Or configure manually:

```bash
cp .env.example .env
# Edit .env — set LLM_PROVIDER and API keys (defaults to AWS Bedrock)
# For 100% local: set LLM_PROVIDER=ollama (see docs/quickstart.md)
docker compose up -d
```

```bash
# Retrieve the auto-generated admin API key
docker compose exec server cat /app/admin-bootstrap-key.txt

# Ingest included sample docs
docker compose exec server docbrain-ingest

# Ask a question
docker compose exec -e DOCBRAIN_API_KEY=<key> server \
  docbrain-cli ask "How do I deploy to production?"
```

Open the Web UI at **http://localhost:3001**.

### Configuration

DocBrain uses a config-first architecture with three layers:

| File | Purpose |
|---|---|
| `config/default.yaml` | Non-secret defaults — committed, safe to inspect |
| `config/local.yaml` | Credentials and local overrides — gitignored |
| `.env` | Infrastructure secrets: `DATABASE_URL`, API keys, `REDIS_URL`, `OPENSEARCH_URL` |

Environment variables always override config files. See [Configuration Guide](docs/configuration.md) for all options.

### Choose Your LLM Provider

| Provider | Config |
|---|---|
| **AWS Bedrock** | `LLM_PROVIDER=bedrock` — uses existing AWS credentials |
| **Anthropic** | `LLM_PROVIDER=anthropic` — requires `ANTHROPIC_API_KEY` |
| **OpenAI** | `LLM_PROVIDER=openai` — requires `OPENAI_API_KEY` |
| **Ollama** | `LLM_PROVIDER=ollama` — 100% local, no data leaves your machine |

See [Provider Setup](docs/providers.md) for detailed configuration.

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
        RD["Redis<br/><i>sessions · cache · rate limits</i>"]
    end

    subgraph "LLM Providers"
        OL["Ollama"]
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
| Autopilot | Custom | Gap analysis, semantic clustering, draft generation |
| Freshness | Custom | 5-signal scoring, contradiction detection, staleness alerts |
| Storage | PostgreSQL 17, OpenSearch 2.19, Redis 7 | Metadata, vectors, sessions |
| Ingest | Custom | 13+ source connectors, heading-aware chunking, image extraction |

Full architecture documentation: [docs/architecture.md](docs/architecture.md)

---

## How It Works

DocBrain's RAG pipeline adds three layers most implementations skip: **memory**, **freshness awareness**, and **autonomous gap detection**.

```mermaid
graph TB
    Q["Question"] --> IC["Intent Classification"]
    IC --> QR["Query Rewriting"]
    QR --> HS["Hybrid Search<br/><i>k-NN + BM25</i>"]
    QR --> ML["Memory Lookup<br/><i>episodic · semantic · procedural</i>"]
    HS --> CA["Context Assembly"]
    ML --> CA
    CA --> RE["Reference Enrichment<br/><i>fetch linked doc chunks</i>"]
    RE --> FS["Freshness Check"]
    FS --> LLM["LLM Generation<br/><i>streaming, with citations</i>"]
    LLM --> CF{"Confidence?"}
    CF -->|"≥ 85%"| R["Answer + Sources"]
    CF -->|"70–84%"| NF["Not found<br/>(max 2 sentences)"]
    CF -->|"< 70%"| CQ["Clarifying question"]
    R & NF & CQ --> EP["Episode Storage"]
    EP -. "feedback loop" .-> AP["Autopilot"]

    style AP fill:#2563eb,color:#fff
    style FS fill:#059669,color:#fff
    style ML fill:#7c3aed,color:#fff
    style CQ fill:#dc2626,color:#fff
```

### Memory System

| Tier | Purpose | Example |
|------|---------|---------|
| **Working** | Conversation context within a session | "by 'the service' I mean auth-service" |
| **Episodic** | Past Q&A across all users, with feedback | "this was asked before — validated answer exists" |
| **Semantic** | Entity graph — services, teams, dependencies | "auth-service depends on Redis, owned by Platform" |
| **Procedural** | Rules learned from feedback patterns | "for deploy questions, always include the canary step" |

### Documentation Autopilot

Autopilot monitors unanswered questions, negative feedback, and recurring gaps across teams. It clusters these signals semantically, classifies them by documentation type, and drafts missing content grounded in your existing docs.

```
Users ask → Gap detected → Draft generated → Admin reviews → Published → Re-ingested
```

See [Autopilot Guide](docs/autopilot.md) for configuration and details.

---

## Deployment

### Docker Compose (default)

```bash
docker compose up -d
```

Starts the API server, web UI, PostgreSQL, OpenSearch, and Redis. Schema migrations run automatically on boot.

### Kubernetes (Helm)

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.anthropicApiKey=sk-ant-... \
  --set embedding.provider=openai \
  --set embedding.openaiApiKey=sk-...
```

Images default to the chart's `appVersion` (`1.2.0`) — no explicit tag override needed.

See [Kubernetes Guide](docs/kubernetes.md) for production configuration including external databases, SSO, Ingress, Vault integration, and scaling.

---

## Integrations

### MCP (Model Context Protocol)

Use DocBrain as a knowledge source in Claude Code, Cursor, or any MCP-compatible editor.

```bash
claude mcp add docbrain \
  -e DOCBRAIN_API_KEY=db_sk_... \
  -e DOCBRAIN_SERVER_URL=http://localhost:3000 \
  -- npx -y docbrain-mcp@latest
```

**Available tools:** `docbrain_ask`, `docbrain_incident`, `docbrain_freshness`, `docbrain_autopilot_gaps`, `docbrain_autopilot_generate`, `docbrain_autopilot_summary`

### Slack

```
/docbrain ask how do we deploy to production?
/docbrain incident payments service 502 after deploy
/docbrain capture          ← inside a thread to index it instantly
```

Full setup guide: [docs/slack.md](docs/slack.md)

### CLI

```bash
brew install docbrain-ai/tap/docbrain
docbrain login --server https://docbrain.mycompany.com
docbrain ask "How do I configure mTLS between services?"
```

---

## Scope

### In Scope

DocBrain is intended to be a self-improving documentation intelligence platform. As such, the project implements:

- Multi-source knowledge ingestion (13+ connectors)
- Confidence-scored retrieval-augmented generation with source attribution
- Multi-tier persistent memory (working, episodic, semantic, procedural)
- Autonomous documentation gap detection and draft generation (Autopilot)
- Document health scoring and staleness alerting
- Real-time knowledge capture from Slack, GitHub, and GitLab
- Multi-tenant space isolation and RBAC
- MCP, Slack, CLI, and Web UI interfaces
- Knowledge Graph with BFS/DFS traversal, blast radius analysis, and expertise routing
- Documentation Analytics: gap resolution rate, knowledge half-life, tribal knowledge %, ROI tracking
- Predictive Intelligence: cascade staleness, seasonal patterns, onboarding gaps, code-change-triggered review
- Autonomous Document Maintenance: AI fix proposals for contradictions, broken links, and version drift
- Knowledge Stream: proactive incident warnings, decay alerts, expertise gap detection
- Embedding model fine-tuning pipeline (opt-in Tier 2) with ONNX export and safety gates

### Out of Scope

DocBrain is designed to work alongside existing tools. The following are explicitly not goals:

- Replacing Confluence, Notion, or other documentation platforms (DocBrain augments them)
- General-purpose LLM chat or code generation
- Real-time collaborative document editing
- Source code analysis or static analysis

---

## Documentation

| | |
|---|---|
| [Quickstart](docs/quickstart.md) | Running locally or in the cloud in 5 minutes |
| [Ingestion Guide](docs/ingestion.md) | Connecting all 13+ knowledge sources |
| [Configuration](docs/configuration.md) | All environment variables and options |
| [Provider Setup](docs/providers.md) | LLM and embedding provider configuration |
| [Architecture](docs/architecture.md) | System design, data flow, memory, freshness, and Autopilot |
| [Autopilot](docs/autopilot.md) | Gap detection, draft generation, and the feedback loop |
| [Knowledge Intelligence](docs/knowledge-intelligence.md) | Knowledge Graph, Documentation Analytics, Predictive Intelligence, Autonomous Maintenance, Knowledge Stream |
| [Learning Pipeline](docs/learning.md) | Embedding fine-tuning — Tier 0/1/2, trainer sidecar, safety mechanisms |
| [API Reference](docs/api-reference.md) | REST API with Autopilot endpoints |
| [RBAC](docs/rbac.md) | Role-based access control |
| [Slack Integration](docs/slack.md) | Slash commands, feedback buttons, and proactive notifications |
| [GitLab Capture](docs/gitlab-capture.md) | Real-time MR discussion indexing |
| [Kubernetes](docs/kubernetes.md) | Helm chart deployment and scaling |
| [Threat Model](THREAT_MODEL.md) | Security analysis: assets, trust boundaries, mitigations |

---

## Community

- **GitHub Issues:** [Bug reports and feature requests](https://github.com/docbrain-ai/docbrain/issues)
- **GitHub Discussions:** [Questions and community conversation](https://github.com/docbrain-ai/docbrain/discussions)
- **Email:** [hello@docbrain.ai](mailto:hello@docbrain.ai)

---

## Contributing

We welcome contributions from the community. Since source code is not yet published, current contributions focus on documentation, configuration, and feedback. Please read our [Contributing Guide](CONTRIBUTING.md) for details on:

- Reporting bugs and requesting features
- Submitting documentation and configuration improvements
- Pull request process

---

## Security

To report a security vulnerability, please follow the process outlined in [SECURITY.md](SECURITY.md). Do **not** file a public GitHub issue for security vulnerabilities.

---

## License

DocBrain is licensed under the [Business Source License 1.1](LICENSE) (BSL 1.1).

- **Production use** is permitted, except offering DocBrain as a hosted/managed service to third parties.
- **Change date:** The earlier of January 1, 2028, or when the repository reaches 5,000 GitHub stars, at which point the license converts to [Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).
- For alternative licensing, contact [licensing@docbrainapi.com](mailto:licensing@docbrainapi.com).

---

## Code of Conduct

This project follows the [Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md). By participating, you are expected to uphold this code. Please report unacceptable behavior to [conduct@docbrain.ai](mailto:conduct@docbrain.ai).
