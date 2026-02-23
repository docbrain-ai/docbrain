# Deployment Configuration Reference

## Manifest Format

All deployments are defined as YAML manifests:

```yaml
apiVersion: acme/v1
kind: Deployment
metadata:
  name: my-service
  team: backend
  environment: production
spec:
  image: registry.acme.internal/my-service:v2.4.1
  replicas: 3
  resources:
    cpu: "500m"
    memory: "512Mi"
  healthCheck:
    path: /health
    interval: 10s
    timeout: 5s
  autoscaling:
    enabled: true
    minReplicas: 2
    maxReplicas: 10
    targetCPU: 70
```

## Environment Variables

Environment-specific configuration is injected via ConfigMaps:

```yaml
env:
  - name: DATABASE_URL
    valueFrom:
      configMapRef: my-service-config
      key: database_url
  - name: API_SECRET
    valueFrom:
      secretRef: my-service-secrets
      key: api_secret
```

## Rolling Updates

By default, deployments use a rolling update strategy:

- **maxSurge**: 25% — allows 25% more pods during rollout
- **maxUnavailable**: 0 — zero-downtime guarantee
- **progressDeadline**: 10m — rollback if not healthy within 10 minutes

## Canary Deployments

For high-risk changes, use canary mode:

```yaml
spec:
  strategy:
    type: canary
    canary:
      weight: 10        # 10% of traffic
      duration: 30m     # observation window
      metrics:
        - name: error_rate
          threshold: 0.01
        - name: p99_latency_ms
          threshold: 500
```

## Rollback

```bash
# Immediate rollback to previous version
acme deploy rollback my-service

# Rollback to specific version
acme deploy rollback my-service --to-revision 42
```

## Common Issues

### ImagePullBackOff
Your container registry credentials may have expired. Refresh them:
```bash
acme registry login
```

### CrashLoopBackOff
Check application logs: `acme logs my-service --tail 100`
Common causes: missing environment variables, database connection failures.

### Insufficient Resources
Request a resource quota increase via the Admin Console under **Settings > Quotas**.
