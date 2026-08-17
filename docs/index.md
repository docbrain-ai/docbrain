---
hide:
  - navigation
  - toc
---

# DocBrain Documentation

**Organizational memory is lost at the source. That is where DocBrain captures it.**

DocBrain captures what your organization learns at the moment it is learned — from tickets, threads, incidents, support conversations and code changes — then scores it, routes it through review, and keeps it accurate as reality moves. Every answer cites the claim it came from, and says so plainly when your record has no answer.

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

Other tools help you search what was already documented. DocBrain captures what your organization actually learned — the decisions, the caveats, the procedures and the context around them — from PRs, Slack threads, CI pipelines, and IDE sessions, and flags that knowledge when something newer contradicts it.

**Documentation is one output. Organizational memory is the product.**

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
| [External Connectors](connectors.md) | Build custom connectors for any knowledge source |
| [Governance](governance.md) | Space ownership, SLAs, breach detection, dashboards |
| [Review Workflows](reviews.md) | Multi-stage approval pipelines for documentation |
| [Knowledge Intelligence](knowledge-intelligence.md) | Graph, analytics, predictive intelligence |
| [Autopilot](autopilot.md) | Gap detection and draft generation |
| [Slack](slack.md) | Slash commands, message shortcuts, and thread capture |
| [Kubernetes](kubernetes.md) | Helm chart production deployment |
| [API Reference](api-reference.md) | Full REST API documentation |
