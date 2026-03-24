---
hide:
  - navigation
  - toc
---

# DocBrain Documentation

**Capture knowledge at the source. Ship docs that stay accurate.**

DocBrain captures knowledge the moment it's created — from PRs, conversations, CI pipelines, and IDE annotations — then scores, reviews, and publishes it automatically.

<div class="grid cards" markdown>

-   :material-rocket-launch:{ .lg .middle } **Quickstart**

    ---

    Get DocBrain running locally in under 5 minutes.

    [:octicons-arrow-right-24: Get started](quickstart.md)

-   :material-cog:{ .lg .middle } **Configuration**

    ---

    Environment variables, YAML config, and secrets management.

    [:octicons-arrow-right-24: Configure](configuration.md)

-   :material-brain:{ .lg .middle } **Architecture**

    ---

    System design, data flow, quality pipeline, and storage.

    [:octicons-arrow-right-24: Learn more](architecture.md)

-   :material-api:{ .lg .middle } **API Reference**

    ---

    Full REST API documentation with request/response schemas.

    [:octicons-arrow-right-24: API docs](api-reference.md)

-   :material-kubernetes:{ .lg .middle } **Kubernetes**

    ---

    Helm chart deployment for production environments.

    [:octicons-arrow-right-24: Deploy](kubernetes.md)

-   :material-shield-lock:{ .lg .middle } **RBAC & SSO**

    ---

    GitHub, GitLab, and OIDC SSO with role-based access control.

    [:octicons-arrow-right-24: Security](rbac.md)

</div>

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
| [Knowledge Intelligence](knowledge-intelligence.md) | Graph, analytics, predictive intelligence |
| [Autopilot](autopilot.md) | Gap detection and draft generation |
| [Slack](slack.md) | Slash commands and real-time capture |
| [Kubernetes](kubernetes.md) | Helm chart production deployment |
| [API Reference](api-reference.md) | Full REST API documentation |
