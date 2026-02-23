# Engineering Onboarding Guide

## First Week

### Day 1: Setup
- [ ] Get laptop and accounts provisioned (IT ticket should be pre-created)
- [ ] Install required tools: Docker, kubectl, acme-cli, VS Code
- [ ] Clone the monorepo: `git clone git@github.com:acme/platform.git`
- [ ] Run `make setup` to configure local development environment
- [ ] Verify: `acme status` should return "Connected to staging"

### Day 2: Architecture Overview
- Read [Getting Started](./getting-started.md) and [Deployment Config](./deployment-config.md)
- Review the architecture diagram in Confluence (Platform > Architecture)
- Shadow a deployment with your onboarding buddy

### Day 3-5: First Contribution
- Pick a "good first issue" from the backlog
- Follow the PR process: branch from main, tests must pass, 1 approval required
- Deploy to staging using `acme deploy apply -f manifest.yaml --env staging`

## Development Workflow

### Local Development
```bash
# Start dependencies
docker compose up -d postgres redis opensearch

# Run the service
cargo run --bin my-service

# Run tests
cargo test --workspace
```

### Code Review Standards
- All PRs require at least 1 approval
- Tests must pass in CI
- No `TODO` comments without a linked ticket
- Security-sensitive changes require security team review

## Key Contacts

| Role | Person | Slack |
|------|--------|-------|
| Engineering Manager | Sarah Chen | @sarah.chen |
| Tech Lead (Backend) | Marcus Rivera | @marcus |
| Tech Lead (Platform) | Priya Patel | @priya |
| Security | Alex Kim | @alex.kim |
| On-call schedule | — | #oncall-schedule |

## Useful Links

- CI/CD Dashboard: https://ci.acme-platform.internal
- Grafana: https://grafana.acme-platform.internal
- Runbooks: https://wiki.acme-platform.internal/runbooks
- API Docs: https://api-docs.acme-platform.internal
