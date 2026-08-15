# Contributing to DocBrain

Thank you for your interest in contributing to DocBrain. This document provides guidelines and information for contributors.

> **Note:** The client tooling source ([`crates/docbrain-cli`](crates/docbrain-cli), [`crates/docbrain-mcp`](crates/docbrain-mcp)) is published in this repo and accepts code PRs — build with `cargo build --workspace`. The DocBrain server is distributed as pre-built Docker images and deployment artifacts; its source is not yet published. Server-side contributions are welcome as documentation, configuration, Helm charts, and bug reports against the published artifacts.

## How to Contribute

### Reporting Bugs

Before filing a bug report, please check [existing issues](https://github.com/docbrain-ai/docbrain/issues) to avoid duplicates.

When filing a bug report, include:

- **DocBrain version** (from `docker compose exec server docbrain --version` or the release tag)
- **Environment** (OS, Docker version, Kubernetes version if applicable)
- **LLM provider and model** in use
- **Steps to reproduce** the issue
- **Expected behavior** vs. **actual behavior**
- **Relevant logs** (redact any API keys or sensitive data)

### Requesting Features

Feature requests are welcome. Please open an issue with:

- A clear description of the problem the feature would solve
- Your proposed solution (if any)
- Any alternatives you have considered

### Documentation Improvements

Documentation improvements are highly valued. You can contribute by:

- Fixing typos, broken links, or unclear instructions
- Adding examples or use cases
- Improving configuration guides
- Translating documentation

### Submitting Changes

1. **Fork** the repository and create a feature branch from `main`
2. **Make your changes** with clear, descriptive commit messages
3. **Verify your changes** — for docs, check that links resolve and Markdown renders correctly; for Helm/config changes, validate with `helm lint` or `docker compose config`
4. **Submit a pull request** with a description of what changed and why

#### Commit Messages

Follow conventional commit format:

```
type(scope): description

[optional body]
```

Types: `docs`, `fix`, `chore`, `feat` (for config/Helm changes)

Examples:
- `docs: clarify Slack integration setup steps`
- `docs(ingestion): add Microsoft Teams prerequisites`
- `fix(helm): correct service port in values.yaml`
- `chore: update .env.example with new provider options`

#### Pull Request Process

1. Ensure your PR description clearly explains the change and links to any related issues
2. Update documentation if your change affects user-facing behavior
3. A maintainer will review your PR and may request changes
4. Once approved, a maintainer will merge your PR

## Local Deployment

To run DocBrain locally for testing documentation or configuration changes:

```bash
git clone https://github.com/docbrain-ai/docbrain.git && cd docbrain
cp .env.example .env
# Edit .env with your configuration
docker compose up -d
```

See the [Quickstart Guide](docs/quickstart.md) for detailed setup instructions.

## Code of Conduct

All contributors are expected to follow our [Code of Conduct](CODE_OF_CONDUCT.md). Please report unacceptable behavior to [conduct@docbrainapi.com](mailto:conduct@docbrainapi.com).

## License

By contributing to DocBrain, you agree that your contributions will be licensed under the [MIT License](LICENSE) that covers this repository. The DocBrain server binaries and container images are distributed separately under the [Business Source License 1.1](LICENSE-SERVER).

## Questions?

If you have questions about contributing, please open a [GitHub Discussion](https://github.com/docbrain-ai/docbrain/discussions) or reach out at [hello@docbrainapi.com](mailto:hello@docbrainapi.com).
