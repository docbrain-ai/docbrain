# CI/CD Pipeline Guide

## Overview

Acme Platform uses GitHub Actions for continuous integration and ArgoCD for continuous deployment. Every merge to `main` triggers a build, test, security scan, and deployment to staging. Production deployments require manual approval.

## Pipeline Stages

```
Push → Build → Test → Security Scan → Container Build → Push to ECR → Deploy Staging → [Manual Gate] → Deploy Production
```

### Stage Details

| Stage | Duration | Tool | Failure Rate |
|-------|----------|------|-------------|
| Build | ~2 min | Cargo/Node | < 1% |
| Unit Tests | ~5 min | cargo test / jest | ~3% |
| Integration Tests | ~8 min | Docker Compose | ~5% |
| Security Scan | ~3 min | Trivy + Semgrep | ~2% |
| Container Build | ~4 min | Docker buildx | < 1% |
| Deploy Staging | ~3 min | ArgoCD | < 1% |
| Deploy Production | ~5 min | ArgoCD | < 0.5% |

## GitHub Actions Workflow

```yaml
name: CI/CD Pipeline
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

jobs:
  build-and-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Build
        run: cargo build --release
      - name: Test
        run: cargo test --workspace
      - name: Lint
        run: cargo clippy -- -D warnings

  security:
    needs: build-and-test
    runs-on: ubuntu-latest
    steps:
      - name: Trivy scan
        uses: aquasecurity/trivy-action@master
        with:
          scan-type: fs
          severity: HIGH,CRITICAL
      - name: Semgrep
        run: semgrep --config auto --error

  deploy-staging:
    needs: security
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    environment: staging
    steps:
      - name: Build and push container
        run: |
          docker buildx build \
            --tag $ECR_REGISTRY/my-service:${{ github.sha }} \
            --push .
      - name: Deploy to staging
        run: argocd app sync my-service-staging

  deploy-production:
    needs: deploy-staging
    if: github.ref == 'refs/heads/main'
    runs-on: ubuntu-latest
    environment:
      name: production
      url: https://api.acme-platform.com
    steps:
      - name: Deploy to production
        run: argocd app sync my-service-production
```

## Environment Configuration

### Staging
- Cluster: `eks-staging-us-east-1`
- Auto-deployed on every merge to `main`
- Uses reduced replicas (1 per service)
- Connected to staging databases and test payment processors

### Production
- Cluster: `eks-prod-us-east-1` and `eks-prod-us-west-2`
- Requires manual approval from a team lead
- Minimum 3 replicas per service
- Connected to production databases and live payment processors

## Rollback Procedure

### Automatic Rollback
ArgoCD monitors health checks after deployment. If the new version fails health checks within 5 minutes, it automatically rolls back.

### Manual Rollback
```bash
# Rollback to previous version
argocd app rollback my-service-production

# Rollback to specific revision
argocd app rollback my-service-production --revision 42

# Emergency: direct kubectl rollback
kubectl rollout undo deployment/my-service -n production
```

## Common CI Failures

### "Insufficient disk space"
Self-hosted runners occasionally run out of disk. The disk cleanup job runs at midnight ET. If urgent:
```bash
docker system prune -af
```

### "ECR push timeout"
ECR rate limits apply. Wait 60 seconds and retry. If persistent, check AWS service health.

### "ArgoCD sync failed"
1. Check ArgoCD UI: https://argocd.acme-platform.internal
2. Common causes: invalid YAML, resource quota exceeded, image pull errors
3. Fix and re-push — ArgoCD will auto-sync

## On-Call Notes

- Pipeline alerts go to #ci-alerts Slack channel
- Failed production deploys page the on-call engineer
- Contact: Platform team (Priya Patel) for pipeline infrastructure issues
