# Secret Management Guide

## Overview

Acme Platform uses HashiCorp Vault for all secret management. Secrets are injected into services via the Vault Agent sidecar, which runs alongside every pod in Kubernetes. Never hardcode secrets in code, configuration files, or environment variables.

## Vault Architecture

```
Application Pod
├── App Container
│   └── reads from /vault/secrets/config
└── Vault Agent Sidecar
    └── authenticates via Kubernetes SA → fetches secrets → writes to shared volume
```

## Accessing Secrets

### From Application Code

Secrets are mounted as files at `/vault/secrets/`:

```python
import json

with open('/vault/secrets/config') as f:
    secrets = json.load(f)

db_password = secrets['database_password']
api_key = secrets['stripe_api_key']
```

### From Kubernetes Manifests

Add the Vault Agent annotations:

```yaml
metadata:
  annotations:
    vault.hashicorp.com/agent-inject: "true"
    vault.hashicorp.com/role: "payments-service"
    vault.hashicorp.com/agent-inject-secret-config: "secret/data/payments/production"
    vault.hashicorp.com/agent-inject-template-config: |
      {{- with secret "secret/data/payments/production" -}}
      {
        "database_password": "{{ .Data.data.database_password }}",
        "stripe_api_key": "{{ .Data.data.stripe_api_key }}"
      }
      {{- end -}}
```

## Secret Rotation

### Automated Rotation

Database credentials rotate automatically every 30 days via Vault's dynamic secrets engine:

```bash
# Current credentials (auto-rotated)
vault read database/creds/payments-service
```

### Manual Rotation

For API keys and third-party credentials:

1. Generate new credential with the third-party provider
2. Update in Vault:
   ```bash
   vault kv put secret/payments/production \
     stripe_api_key="sk_live_new_key" \
     database_password="existing_password"
   ```
3. Vault Agent detects the change and restarts the sidecar (takes up to 5 minutes)
4. Verify the application is using the new key
5. Revoke the old key with the third-party provider

### Zero-Downtime Key Rotation

For services that can't tolerate a restart:

1. Add the new key alongside the old key:
   ```bash
   vault kv put secret/payments/production \
     stripe_api_key="sk_live_new_key" \
     stripe_api_key_previous="sk_live_old_key"
   ```
2. Application reads both and tries new first, falls back to old
3. After grace period (24h), remove the old key
4. Revoke the old key

## Access Policies

| Role | Access | Example |
|------|--------|---------|
| Service | Read own secrets only | payments-service → secret/payments/* |
| Developer | Read staging secrets | all → secret/*/staging |
| SRE | Read/write all secrets | all → secret/* |
| Admin | Full access + policy management | all → * |

## Emergency Procedures

### Lost Secret Recovery

If a secret is accidentally deleted:
1. Check Vault's version history: `vault kv get -version=N secret/path`
2. Undelete: `vault kv undelete -versions=N secret/path`
3. If permanently deleted, regenerate from the source system

### Vault Seal Emergency

If Vault becomes sealed (rare but critical):
1. Alert: Vault seal page goes to #security on-call
2. Requires 3 of 5 unseal key holders to unseal
3. Key holders: CTO, VP Engineering, Security Lead, SRE Lead, Platform Lead
4. Unseal procedure: `vault operator unseal $UNSEAL_KEY` (repeat 3 times with different keys)

## Audit

All secret access is logged. View audit logs:
```bash
vault audit list
# Logs are stored in S3: s3://acme-vault-audit/
```

## Contacts

- Vault administration: #security on Slack
- Secret access requests: File a Jira ticket in the SEC project
- Emergency unseal: Page @security-oncall
