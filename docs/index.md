---
hide:
  - navigation
  - toc
---

# DocBrain Documentation

**Stop writing docs after the fact. Capture knowledge where it happens.**

DocBrain intercepts knowledge at the moment of creation — from PRs, Slack threads, CI pipelines, and IDE sessions — then scores, reviews, and publishes it before anyone has to ask "where's the doc for this?"

---

<div class="doc-grid" markdown>

<div class="doc-card" markdown>
### :material-rocket-launch: Quickstart
Get DocBrain running locally in under 5 minutes.

[:material-arrow-right: Get started](quickstart.md)
</div>

<div class="doc-card" markdown>
### :material-cog: Configuration
Environment variables, YAML config, and secrets management.

[:material-arrow-right: Configure](configuration.md)
</div>

<div class="doc-card" markdown>
### :material-brain: Architecture
System design, data flow, quality pipeline, and storage.

[:material-arrow-right: Learn more](architecture.md)
</div>

<div class="doc-card" markdown>
### :material-api: API Reference
Full REST API documentation with request/response schemas.

[:material-arrow-right: API docs](api-reference.md)
</div>

<div class="doc-card" markdown>
### :material-kubernetes: Kubernetes
Helm chart deployment for production environments.

[:material-arrow-right: Deploy](kubernetes.md)
</div>

<div class="doc-card" markdown>
### :material-shield-lock: RBAC & SSO
GitHub, GitLab, and OIDC SSO with role-based access control.

[:material-arrow-right: Security](rbac.md)
</div>

</div>

---

## What makes DocBrain different?

Most documentation tools index existing docs and answer questions. DocBrain captures the knowledge that was **never written down** — from PRs, Slack threads, CI pipelines, and IDE sessions — and turns it into documentation that meets your team's quality standards.

```
Developer merges a PR           → DocBrain extracts decisions, caveats, procedures
Team discusses in Slack          → DocBrain distills fragments from the conversation
CI pipeline deploys              → DocBrain captures deployment context and changes
Engineer annotates in their IDE  → DocBrain links knowledge to the exact code location

     Fragments accumulate → Quality scored → Clusters detected → Docs composed
                                                                      ↓
                              Review workflow → Style checks → Published
```

## Explore

| Guide | Description |
|-------|-------------|
| [Quickstart](quickstart.md) | Running locally in 5 minutes |
| [Configuration](configuration.md) | All environment variables and options |
| [LLM Providers](providers.md) | 14 supported providers including fully-local Ollama |
| [Ingestion](ingestion.md) | Connecting 13+ knowledge sources |
| [Governance](governance.md) | Space ownership, SLAs, breach detection, dashboards |
| [Review Workflows](reviews.md) | Multi-stage approval pipelines for documentation |
| [Knowledge Intelligence](knowledge-intelligence.md) | Graph, analytics, predictive intelligence |
| [Autopilot](autopilot.md) | Gap detection and draft generation |
| [Slack](slack.md) | Slash commands and real-time capture |
| [Kubernetes](kubernetes.md) | Helm chart production deployment |
| [API Reference](api-reference.md) | Full REST API documentation |
