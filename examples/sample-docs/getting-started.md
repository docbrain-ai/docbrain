# Getting Started with Acme Platform

## Overview

Acme Platform is a cloud-native application deployment system that simplifies container orchestration, CI/CD pipelines, and observability for engineering teams.

## Architecture

The platform consists of three main components:

- **Control Plane** — manages cluster state, scheduling, and API gateway
- **Data Plane** — runs workloads across availability zones
- **Observability Stack** — metrics, logs, and distributed tracing

## Authentication

All API calls require a bearer token. Generate one from the Admin Console:

1. Navigate to **Settings > API Keys**
2. Click **Generate New Key**
3. Select the appropriate scope (read-only or read-write)
4. Copy the key — it will not be shown again

```bash
curl -H "Authorization: Bearer ak_your_key_here" \
  https://api.acme-platform.internal/v1/deployments
```

## Deployment Workflow

1. Push code to your repository
2. The CI pipeline builds and tags the container image
3. Create a deployment manifest (see [Deployment Configuration](./deployment-config.md))
4. Apply via CLI: `acme deploy apply -f manifest.yaml`
5. Monitor rollout: `acme deploy status my-service`

## Rate Limits

| Tier       | Requests/min | Burst |
|------------|-------------|-------|
| Free       | 60          | 10    |
| Team       | 600         | 100   |
| Enterprise | 6,000       | 1,000 |

## Support

- Slack: #acme-platform-support
- On-call: PagerDuty rotation "Platform Engineering"
- Runbook: [Incident Playbook](./incident-playbook.md)
