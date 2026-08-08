<p align="center">
  <img src="assets/banner.png" alt="DocBrain" width="600" />
</p>

<p align="center">
  <strong>The institutional memory layer for your organization.</strong><br/>
  DocBrain captures the knowledge your teams create in PRs, chat threads, deploys, and incidents — turns it into documentation with per-claim citations, and keeps it accurate as reality changes. Self-hosted. Read-only. Every answer cited to its source.
</p>

<p align="center">
  <a href="https://docbrainapi.com"><img src="https://img.shields.io/badge/website-docbrainapi.com-6366f1" alt="Website" /></a>
  <a href="https://github.com/docbrain-ai/docbrain/stargazers"><img src="https://img.shields.io/github/stars/docbrain-ai/docbrain?style=social" alt="Stars" /></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-BSL%201.1-blue" alt="License" /></a>
  <img src="https://img.shields.io/badge/built_with-Rust-orange" alt="Rust" />
  <a href="https://github.com/docbrain-ai/docbrain/actions/workflows/clients.yml"><img src="https://github.com/docbrain-ai/docbrain/actions/workflows/clients.yml/badge.svg" alt="clients CI" /></a>
  <a href="https://glama.ai/mcp/servers/docbrain-ai/docbrain"><img src="https://glama.ai/mcp/servers/docbrain-ai/docbrain/badge" alt="MCP" height="20" /></a>
</p>

<p align="center">
  <a href="https://docbrainapi.com"><strong>Website</strong></a> &bull;
  <a href="https://docbrainapi.com/docs"><strong>Docs</strong></a> &bull;
  <a href="#quickstart">Quickstart</a> &bull;
  <a href="#the-problem">The Problem</a> &bull;
  <a href="#how-docbrain-works">How It Works</a> &bull;
  <a href="#architecture">Architecture</a> &bull;
  <a href="#security">Security</a>
</p>

---

<p align="center">
  <a href="https://youtu.be/zZ7WdjmOXHU">
    <img src="assets/quickstart-preview.gif" alt="docbrain ask answering with per-claim citations — click to watch the full unedited quickstart" width="720" />
  </a>
  <br/>
  <em>real recording, no edits — <a href="https://youtu.be/zZ7WdjmOXHU">watch the full 90-second quickstart ▶</a></em>
</p>

---

> **Project Status: client source open, server closed.** The source for everything DocBrain runs on *your* side of the network boundary — the `docbrain` CLI and the IDE MCP server — is in this repo under [`crates/`](crates/), MIT-licensed, built and tested in public CI. Audit exactly what runs in your environment and what leaves it. The server ships as free production Docker images (BSL 1.1 permits production use) with full Helm charts, complete configuration, the [threat model](THREAT_MODEL.md), and all the docs to self-host in production. The server source stays closed for now — we originally targeted the first half of 2026 to publish it, we missed that date, and we won't post a new date until we're certain we can hit it. If a closed server is a dealbreaker for you, that's a rational position and we respect it: [how DocBrain earns trust](https://docbrainapi.com/docs/trust/). Contributions: code PRs for the client crates, plus documentation, configuration, and bug reports. When the server source publishes, it will be under the BSL 1.1 terms below.

---

## The Problem

Every organization runs on knowledge that never gets written down: the decision from a meeting, the fix someone found at 2am, the workaround only one person knows. It lives in PRs, chat threads, tickets, and people's heads. When that person changes teams or quits, years of context walk out the door with them.

Tools that "index your docs and add a chatbot" solve the wrong half of the problem: they retrieve your stale, incomplete wiki slightly faster. The knowledge that actually runs your organization was never captured in the first place. And it's getting worse now that AI produces code, changes, and fluent documentation faster than any human can absorb — your agents read those docs too. More documentation is easy. Documentation your organization can trust is the scarce thing.

## How DocBrain Works

DocBrain captures knowledge **at the source**, the moment it's created:

```
  Someone merges a change      ──→  decisions, caveats, procedures extracted
  A team works through chat    ──→  the answer, distilled from the thread
  A deploy goes out            ──→  what changed and why
  On-call resolves an incident ──→  the fix and the root cause
  Any other system you run     ──→  ingested via the Connector SDK
```

Captured fragments are confidence-scored, connected into one memory, and composed into documentation with **per-claim provenance**. Drafts route through human review before anything publishes. Then DocBrain keeps the result honest: freshness tracking, contradiction detection, and staleness alerts as reality changes. Ask a question, get a cited answer — or an honest "I don't know" instead of a guess.

## Quickstart

```bash
git clone https://github.com/docbrain-ai/docbrain.git && cd docbrain
./scripts/setup.sh    # interactive wizard: picks provider, sets keys, starts services
```

Or manually:

```bash
cp .env.example .env   # set LLM_PROVIDER and API keys
docker compose up -d
```

```bash
# Get the auto-generated admin API key
docker compose exec server cat /app/admin-bootstrap-key.txt

# Open the web dashboard
open http://localhost:3001

# Or ask a question via API
curl -H "Authorization: Bearer <key>" \
     -H "Content-Type: application/json" \
     -d '{"question":"How do I deploy to production?"}' \
     http://localhost:3001/api/v1/ask
```

Full setup guide: [docs/quickstart.md](docs/quickstart.md)

## What You Get

- **Capture from 13 built-in sources** — Confluence, Slack, Teams, GitHub, GitLab, Jira, PagerDuty, Linear, OpsGenie, Rootly, Zendesk, Intercom, and local files, plus a language-agnostic [Connector SDK](docs/connectors.md) for anything else. [Ingestion guide →](docs/ingestion.md)
- **Ask, with citations** — hybrid vector + keyword search, confidence-scored answers; low confidence asks clarifying questions instead of guessing. [API →](docs/api-reference.md)
- **`docbrain generate`** — on-demand docs grounded in your own runbooks, incidents, threads, and PRs, with per-claim provenance and honest `needs_input` for what the knowledge can't answer. [Generate guide →](docs/generate.md)
- **Quality gates on every doc** — structural, style (your style guide, enforced automatically), and semantic scoring; nothing unscored enters the system. [Style policy →](docs/style-policy.md)
- **Review workflows and ownership** — multi-stage approvals, space owners, SLAs, and governance dashboards, so documentation has accountability. [Governance →](docs/governance.md) · [Reviews →](docs/reviews.md)
- **Autopilot** — clusters unanswered questions into gaps, drafts grounded fixes, and routes them to human review. Nothing publishes without oversight. [Autopilot →](docs/autopilot.md)
- **Freshness and contradiction detection** — stale docs flagged, conflicting docs surfaced, cascade staleness traced across dependent docs. [Knowledge intelligence →](docs/knowledge-intelligence.md)
- **Source-system ACL mirroring** — Confluence restrictions, Slack channel membership, and repo visibility enforced at query time. [Access control →](docs/access-control.md)
- **RBAC, SSO, audit logging** — 4-tier roles, GitHub/GitLab/OIDC SSO, per-space isolation. [RBAC →](docs/rbac.md)
- **Everywhere your team works** — web dashboard, Slack commands, CLI, CI hooks, and MCP tools for Claude Code, Cursor, and any MCP-compatible editor. [Slack →](docs/slack.md)

## Architecture

```mermaid
graph TB
    subgraph "Capture Layer"
        CI["CI/CD Pipelines"]
        IDE["IDE (MCP)"]
        SLACK["Slack / Teams"]
        WEB["Web UI"]
        CLI["CLI"]
        API_EXT["External APIs"]
    end

    subgraph "DocBrain Server (Rust / Axum)"
        FRAG["Fragment Router"]
        QUAL["Quality Pipeline"]
        CLUST["Clustering Engine"]
        COMP["Composition Engine"]
        REV["Review Workflows"]
        RAG["RAG Pipeline"]
        AUTO["Autopilot"]
        GOV["Governance"]
        EVT["Event Bus + Webhooks"]
    end

    subgraph "Storage"
        PG["PostgreSQL"]
        OS["OpenSearch<br/><i>vector + keyword</i>"]
        RD["Redis"]
    end

    CI & IDE & SLACK & WEB & CLI & API_EXT --> FRAG
    FRAG --> QUAL --> CLUST --> COMP --> REV
    WEB & CLI & SLACK --> RAG
    RAG & AUTO & GOV --> PG & OS
    EVT --> PG
```

Rust server, PostgreSQL, OpenSearch, Redis. Full design: [docs/architecture.md](docs/architecture.md)

## Security

DocBrain runs entirely in your infrastructure, **read-only** against your sources. You choose where the model runs: fully local via Ollama (zero egress), your own cloud account (Bedrock, Azure, Vertex — your KMS, your audit trail), or a provider API. Documents, embeddings, and indexes never leave your network; only the query and the relevant chunks reach the LLM you chose.

API keys are Argon2-hashed, every endpoint enforces RBAC, rate limits are per-key, and admin actions are audit-logged. The client code you install is open source in [`crates/`](crates/). The full threat model — 11 analyzed attack vectors and an operator checklist — is published: [THREAT_MODEL.md](THREAT_MODEL.md)

**LLM providers (14):** Anthropic, OpenAI, AWS Bedrock, Ollama, Google Gemini, Vertex AI, Azure OpenAI, DeepSeek, Groq, Mistral, xAI, OpenRouter, Together AI, Cohere. [Provider setup →](docs/providers.md)

## Deployment

```bash
# Docker Compose — everything behind a single origin at localhost:3001
docker compose up -d

# Kubernetes
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.anthropicApiKey=sk-ant-...
```

[Kubernetes guide →](docs/kubernetes.md) · [Configuration →](docs/configuration.md)

## Documentation

| | |
|---|---|
| [Quickstart](docs/quickstart.md) | Running locally in 5 minutes |
| [Configuration](docs/configuration.md) | All environment variables and options |
| [Provider Setup](docs/providers.md) | LLM and embedding provider configuration |
| [Architecture](docs/architecture.md) | System design, data flow, memory, freshness |
| [Ingestion Guide](docs/ingestion.md) | Connecting the 13 built-in knowledge sources |
| [External Connectors](docs/connectors.md) | Build custom connectors for any knowledge source |
| [Governance](docs/governance.md) | Ownership, SLAs, breach detection, dashboards |
| [Review Workflows](docs/reviews.md) | Multi-stage approval pipelines |
| [Knowledge Intelligence](docs/knowledge-intelligence.md) | Graph, analytics, predictive intelligence |
| [Autopilot](docs/autopilot.md) | Gap detection, draft generation, feedback loop |
| [Generate](docs/generate.md) | Grounded on-demand doc generation |
| [API Reference](docs/api-reference.md) | Full REST API documentation |
| [RBAC](docs/rbac.md) | Role-based access control and SSO |
| [Slack Integration](docs/slack.md) | Slash commands, message shortcuts, thread capture |
| [Kubernetes](docs/kubernetes.md) | Helm chart deployment |

## See It In Action

**▶ [The quickstart, recorded unedited](https://youtu.be/zZ7WdjmOXHU)** — install → ingest → cited answer → `generate` turning raw on-call notes into a runbook that cites its sources. 90 seconds, shipped images, 100% local models, nothing staged.

| | |
|---|---|
| [What is DocBrain?](https://youtu.be/S4aSTmevvOQ), 5-min overview | [Deep Dive Podcast](https://youtu.be/GN4SC6L8YmI), 20-min deep dive |
| [MCP Preview](https://youtu.be/9mZLoQnGLl8), 30-sec IDE demo | [Full Proof Demo](https://youtu.be/yqj5BCVOLHw), Downvote → Gap → Draft |

## Community

- **GitHub Issues:** [Bug reports and feature requests](https://github.com/docbrain-ai/docbrain/issues)
- **GitHub Discussions:** [Questions and community conversation](https://github.com/docbrain-ai/docbrain/discussions)
- **Email:** [hello@docbrainapi.com](mailto:hello@docbrainapi.com)

## Contributing

We welcome contributions. The client tooling source ([`crates/`](crates/)) accepts code PRs; server-side contributions land best as documentation, configuration, and bug reports. See [Contributing Guide](CONTRIBUTING.md).

## Security Reports

To report a security vulnerability, see [SECURITY.md](SECURITY.md). Do **not** file a public issue.

## License

[Business Source License 1.1](LICENSE) (BSL 1.1). Production use is permitted, except offering DocBrain as a hosted service. Converts to [Apache 2.0](https://www.apache.org/licenses/LICENSE-2.0) on January 1, 2028. For alternative licensing: [licensing@docbrainapi.com](mailto:licensing@docbrainapi.com).

## Code of Conduct

[Contributor Covenant Code of Conduct](CODE_OF_CONDUCT.md).
Report concerns to [hello@docbrainapi.com](mailto:hello@docbrainapi.com).
