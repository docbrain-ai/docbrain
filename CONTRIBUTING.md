# Contributing to DocBrain

Thank you for your interest in contributing to DocBrain. This document provides guidelines and information for contributors.

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
3. **Test your changes** thoroughly
4. **Submit a pull request** with a description of what changed and why

#### Commit Messages

Follow conventional commit format:

```
type(scope): description

[optional body]
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `chore`

Examples:
- `docs: clarify Slack integration setup steps`
- `fix(ingestion): handle empty Confluence pages gracefully`
- `feat(autopilot): add severity threshold configuration`

#### Pull Request Process

1. Ensure your PR description clearly explains the change and links to any related issues
2. Update documentation if your change affects user-facing behavior
3. A maintainer will review your PR and may request changes
4. Once approved, a maintainer will merge your PR

## Development Setup

### Prerequisites

- Docker and Docker Compose
- An LLM API key or Ollama for local inference

### Running Locally

```bash
git clone https://github.com/docbrain-ai/docbrain.git && cd docbrain
cp .env.example .env
# Edit .env with your configuration
docker compose up -d
```

See the [Quickstart Guide](docs/quickstart.md) for detailed setup instructions.

## Code of Conduct

All contributors are expected to follow our [Code of Conduct](CODE_OF_CONDUCT.md). Please report unacceptable behavior to [conduct@docbrain.ai](mailto:conduct@docbrain.ai).

## License

By contributing to DocBrain, you agree that your contributions will be licensed under the same [BSL 1.1 License](LICENSE) that covers the project.

## Questions?

If you have questions about contributing, please open a [GitHub Discussion](https://github.com/docbrain-ai/docbrain/discussions) or reach out at [hello@docbrain.ai](mailto:hello@docbrain.ai).
