# Kubernetes Deployment

Deploy DocBrain to Kubernetes using the included Helm chart.

## Prerequisites

- Kubernetes 1.26+
- Helm 3.x
- `kubectl` configured for your cluster

---

## Helm Install

### 1. Clone the repository

```bash
git clone https://github.com/docbrain-ai/docbrain-public.git
cd docbrain-public
```

The chart is in `helm/docbrain/`. You don't need a Helm repository — install directly from the local path.

### 2. Choose your LLM provider

DocBrain supports Anthropic, OpenAI, AWS Bedrock, and Ollama. Pass the provider and credentials at install time.

#### Anthropic (Claude)

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.modelId=claude-sonnet-4-5-20250929 \
  --set llm.anthropicApiKey=sk-ant-... \
  --set embedding.provider=openai \
  --set embedding.modelId=text-embedding-3-small \
  --set embedding.openaiApiKey=sk-...
```

#### OpenAI

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=openai \
  --set llm.modelId=gpt-4o \
  --set llm.openaiApiKey=sk-... \
  --set embedding.provider=openai \
  --set embedding.modelId=text-embedding-3-small
  # embedding.openaiApiKey defaults to llm.openaiApiKey if not set
```

#### AWS Bedrock

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=bedrock \
  --set llm.modelId=us.anthropic.claude-sonnet-4-20250514-v1:0 \
  --set embedding.provider=bedrock \
  --set embedding.modelId=cohere.embed-v4:0
```

Bedrock uses IAM — configure `serviceAccount.annotations` with your IRSA role ARN instead of API keys:

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=bedrock \
  --set "serviceAccount.annotations.eks\.amazonaws\.com/role-arn=arn:aws:iam::123456789012:role/docbrain-bedrock-role"
```

#### Ollama (local/self-hosted)

Basic (e.g. 8B):

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=ollama \
  --set llm.ollamaBaseUrl=http://ollama-service:11434 \
  --set llm.modelId=llama3.1 \
  --set embedding.provider=ollama \
  --set embedding.modelId=nomic-embed-text
```

For **large models (e.g. 70B)** set `llm.fastModelId` so intent/rewrite stay fast, and `llm.ollamaTimeoutSecs` to avoid "error decoding response body" when the model takes longer than 2 minutes:

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=ollama \
  --set llm.ollamaBaseUrl=http://ollama-service:11434 \
  --set llm.modelId=llama3.1:70b \
  --set llm.fastModelId=llama3.1:8b \
  --set llm.ollamaTimeoutSecs=300 \
  --set embedding.provider=ollama \
  --set embedding.modelId=nomic-embed-text
```

### 3. Choose your ingestion source

#### Local documents (default)

The chart creates a PersistentVolumeClaim and mounts it at `ingest.localDocsPath`. Upload your documents to that path.

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.anthropicApiKey=sk-ant-... \
  --set ingest.sources=local \
  --set ingest.localDocsPath=/data/docs \
  --set ingest.docsStorage=10Gi
```

#### Confluence

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.anthropicApiKey=sk-ant-... \
  --set ingest.sources=confluence \
  --set ingest.confluence.baseUrl=https://yourcompany.atlassian.net \
  --set ingest.confluence.email=user@yourcompany.com \
  --set ingest.confluence.apiToken=ATATT... \
  --set ingest.confluence.spaceKeys=ENG,PLAT
```

For specific pages (instead of full spaces):

```bash
  --set ingest.confluence.pageIds=421856743,41912006
```

For Confluence Data Center (self-hosted):

```bash
  --set ingest.confluence.apiVersion=v1 \
  --set ingest.confluence.tlsVerify=false   # if using a private CA
```

#### Multiple sources

Ingestion sources with more than one target (GitHub, GitLab, Slack, Jira,
Linear, etc.) need YAML-shaped configuration because their resource lists
can't be expressed in a single env var. Put them under the `ingest.sources`
subtree in a values override file:

```yaml
# my-values.yaml
llm:
  provider: anthropic
  anthropicApiKey: sk-ant-...

ingest:
  confluence:                 # legacy flat shape — kept for Confluence
    baseUrl: https://yourcompany.atlassian.net
    email: user@yourcompany.com
    apiToken: ATATT...
  sources:                    # rendered into a ConfigMap + mounted at /app/config/local.yaml
    local:
      path: /data/docs
    github:
      token: ${GITHUB_TOKEN}   # from Kubernetes secret env var
      pull_requests:
        repos:
          - acme/platform
          - acme/docs
        lookback_days: 365
    jira:
      base_url: https://yourcompany.atlassian.net
      user_email: bot@yourcompany.com
      api_token: ${JIRA_API_TOKEN}
      projects:
        - ENG
        - PLAT
```

```bash
helm install docbrain ./helm/docbrain -f my-values.yaml
```

Every resource list (`repos`, `projects`, `channels`, `teams`, …) must be
non-empty — an empty list is a startup error. See
[configuration.md](./configuration.md) for the full `sources:` shape and
selector grammar.

Supported sub-source names (these end up as the `source_type` on ingested
documents): `local`, `confluence`, `github`, `github_pr`, `slack_thread`,
`jira`, `linear`, `pagerduty`, `opsgenie`, `zendesk`, `intercom`, `gitlab_mr`.

### 4. Verify the install

```bash
# Check pod status
kubectl get pods -l app.kubernetes.io/instance=docbrain

# Watch rollout
kubectl rollout status deployment/docbrain-server

# Check server logs
kubectl logs deployment/docbrain-server -f

# Get the bootstrap admin key from the Helm-managed Secret
# The secret name is <release-name>-initial-admin-secret (e.g. "docbrain" if you used the default release name)
kubectl get secret docbrain-initial-admin-secret \
  -o jsonpath='{.data.BOOTSTRAP_ADMIN_KEY}' | base64 -d
```

The bootstrap admin key lives in its own Secret, separate from operational credentials. The key is **never printed to logs** — always retrieve it from the Secret directly as shown above.

**Bootstrap key lifecycle:**

| Event | Secret | Database |
|-------|--------|----------|
| `helm install` | Created with generated key | Key registered on first server boot |
| `helm upgrade` | Key preserved via `lookup` — never regenerated | No-op |
| `kubectl delete secret ...-initial-admin-secret` | Gone | Key still active — safe to delete once you have real keys |
| `bootstrapKey.enabled=false` + `helm upgrade` | Helm deletes the Secret | Server revokes key from DB on next restart |
| `helm uninstall` | Deleted | Key record remains in DB (now inactive after revocation, or still active if not) |

**The initial-admin-secret is intentionally separate from the main Secret** (`docbrain-secret` holds DB passwords and API keys). You can delete, inspect, or manage it without touching operational credentials.

**Recommended first-boot flow:**

1. Retrieve the bootstrap key and log in.
2. Create a named admin API key (or configure SSO).
3. Disable the bootstrap key — one command, no manual DB or API calls:

```bash
helm upgrade docbrain ./helm/docbrain --set bootstrapKey.enabled=false
```

This simultaneously:
- Stops rendering `initial-admin-secret` → Helm deletes it on upgrade
- Sets `BOOTSTRAP_KEY_ENABLED=false` in the ConfigMap → server revokes the key from the database on next restart

To verify it's gone:

```bash
kubectl get secret docbrain-initial-admin-secret   # should return NotFound
kubectl logs deployment/docbrain-server | grep -i bootstrap
# → "BOOTSTRAP_KEY_ENABLED=false — revoked 1 bootstrap key(s) from database"
```

### 5. Access the UI

By default, the UI is only accessible within the cluster. Forward the web port to test locally:

```bash
kubectl port-forward svc/docbrain-web 3001:3001
# open http://localhost:3001
```

For production access, enable Ingress — see [Ingress](#ingress) below.

---

## Configuration

### Using a values file (recommended for production)

Instead of many `--set` flags, use a `values.yaml` override file:

```yaml
# my-values.yaml
llm:
  provider: anthropic
  modelId: claude-sonnet-4-5-20250929
  anthropicApiKey: sk-ant-...

embedding:
  provider: openai
  modelId: text-embedding-3-small
  openaiApiKey: sk-...

ingest:
  sources: confluence
  schedule: "0 */6 * * *"
  confluence:
    baseUrl: https://yourcompany.atlassian.net
    email: user@yourcompany.com
    apiToken: ATATT...
    spaceKeys: ENG,PLAT

server:
  corsAllowedOrigins: "https://docbrain.yourcompany.com"

ingress:
  enabled: true
  host: docbrain.yourcompany.com
  tls: true
  tlsSecretName: docbrain-tls
```

```bash
helm install docbrain ./helm/docbrain -f my-values.yaml
helm upgrade docbrain ./helm/docbrain -f my-values.yaml
```

### Using External Databases

By default the chart deploys PostgreSQL, OpenSearch, and Redis in-cluster as StatefulSets. For production, use managed services instead:

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.anthropicApiKey=sk-ant-... \
  --set postgresql.internal=false \
  --set postgresql.externalUrl="postgresql://docbrain:pass@rds-host.us-east-1.rds.amazonaws.com:5432/docbrain" \
  --set opensearch.internal=false \
  --set opensearch.externalUrl="https://my-cluster.us-east-1.es.amazonaws.com:9200" \
  --set redis.internal=false \
  --set redis.externalUrl="redis://my-cluster.cache.amazonaws.com:6379"
```

### Storage classes

If your cluster requires a specific StorageClass (e.g. `gp3` on EKS, `premium-rwo` on GKE):

```bash
helm install docbrain ./helm/docbrain \
  --set postgresql.storageClassName=gp3 \
  --set opensearch.storageClassName=gp3 \
  --set redis.storageClassName=gp3 \
  --set ingest.storageClassName=gp3   # for local docs PVC
```

Leave empty to use the cluster's default StorageClass.

### Using Existing Secrets

To avoid putting secrets in Helm values (recommended for GitOps workflows), create a Secret manually and reference it:

```bash
kubectl create secret generic docbrain-secrets \
  --from-literal=ANTHROPIC_API_KEY=sk-ant-... \
  --from-literal=OPENAI_API_KEY=sk-... \
  --from-literal=POSTGRES_PASSWORD=your-db-password \
  --from-literal=BOOTSTRAP_ADMIN_KEY=db_sk_...  # optional, auto-generated if absent
```

```bash
helm install docbrain ./helm/docbrain \
  --set existingSecret=docbrain-secrets \
  --set llm.provider=anthropic \
  --set embedding.provider=openai
```

When `existingSecret` is set, the chart does **not** create its own Secret.

### Vault / External Secrets

For Vault Agent injection, use `server.vaultAnnotations` and `server.secrets`:

```yaml
server:
  vaultAnnotations:
    vault.hashicorp.com/agent-inject: "true"
    vault.hashicorp.com/role: "docbrain"
    vault.hashicorp.com/agent-inject-secret-config: "secret/docbrain/config"
  secrets:
    ANTHROPIC_API_KEY: ""   # populated by Vault Agent sidecar
```

### Ingress

```bash
helm install docbrain ./helm/docbrain \
  --set ingress.enabled=true \
  --set ingress.className=nginx \
  --set ingress.host=docbrain.yourcompany.com \
  --set ingress.tls=true \
  --set ingress.tlsSecretName=docbrain-tls
```

The Ingress routes `/api/*` to the server (port 3000) and everything else to the web UI (port 3001).

### SSO / OAuth

See [docs/rbac.md](rbac.md) for full SSO setup. Quick example for GitHub OAuth:

```bash
helm install docbrain ./helm/docbrain \
  --set githubOAuth.enabled=true \
  --set githubOAuth.clientId=Iv1.abc123 \
  --set githubOAuth.clientSecret=... \
  --set githubOAuth.redirectUri=https://docbrain.yourcompany.com/api/v1/auth/github/callback \
  --set oidc.webUiUrl=https://docbrain.yourcompany.com
```

### Autopilot (AI gap analysis)

Autopilot is **enabled by default**. To adjust the gap analysis interval (default: 6 hours):

```bash
helm upgrade docbrain ./helm/docbrain \
  --set autopilot.gapAnalysisIntervalHours=24
```

To disable:

```bash
helm upgrade docbrain ./helm/docbrain \
  --set autopilot.enabled=false
```

### Image tags

| Tag | Use case |
|-----|----------|
| `edge` | Latest commit from `main`. Unstable — good for local testing. |
| `latest` | Latest stable release. |
| `1.2.0` | Pinned release — recommended for production. |

By default the chart uses `appVersion` (`1.2.0`) for all DocBrain images — no explicit tag override needed. Override only when pinning to a different release:

```bash
# Pin to a specific older release
helm install docbrain ./helm/docbrain \
  --set server.image.tag=1.1.10 \
  --set web.image.tag=1.1.10

# Use the latest unstable edge build
helm install docbrain ./helm/docbrain \
  --set server.image.tag=edge \
  --set server.image.pullPolicy=Always \
  --set web.image.tag=edge \
  --set web.image.pullPolicy=Always
```

---

## Upgrade

```bash
helm upgrade docbrain ./helm/docbrain -f my-values.yaml
```

Rolling restarts happen automatically when ConfigMap or Secret values change (via checksum annotations on the pod template).

---

## Uninstall

```bash
helm uninstall docbrain
```

PersistentVolumeClaims are **not** deleted by default (they are annotated with `helm.sh/resource-policy: keep`). To remove them manually:

```bash
kubectl delete pvc -l app.kubernetes.io/instance=docbrain
```

---

## Monitoring

The server exposes a `/health` endpoint used by Kubernetes readiness and liveness probes. For deeper monitoring:

- **Logs**: `kubectl logs deployment/docbrain-server -f` — structured JSON (set `LOG_FORMAT=json`) or human-readable compact format
- **Metrics**: Set `server.env.RUST_LOG=info` for timing logs on each RAG pipeline phase
- **Ingestion**: Monitor the CronJob for failures: `kubectl get cronjob docbrain-ingest`

---

## Scaling

- **API Server**: Increase `server.replicas` for horizontal scaling. Sessions are stored in Redis so any replica can serve any request.
- **OpenSearch**: For large document sets, use a managed OpenSearch cluster (set `opensearch.internal=false`).
- **Ingestion**: The CronJob runs as a single instance (`concurrencyPolicy: Forbid`) — this is intentional to prevent duplicate indexing.

---

## Troubleshooting

### Server crashes with no log output

The server logs are available even on crash. If you see `CrashLoopBackOff` with empty logs, check init containers first:

```bash
kubectl describe pod -l app.kubernetes.io/name=docbrain
kubectl logs <pod-name> -c wait-for-postgres
kubectl logs <pod-name> -c wait-for-opensearch
```

Then check the main container:

```bash
kubectl logs <pod-name> --previous
```

### Config not loading

The server reads config from `/app/config/` inside the container (set via `DOCBRAIN_CONFIG_DIR`). If you see an error about the config directory, ensure `DOCBRAIN_CONFIG_DIR` is in the ConfigMap (it is by default in chart version ≥ 0.2).

### OpenSearch StatefulSet patch fails

If you change OpenSearch probe settings and get a "field is immutable" error, delete the StatefulSet with orphan cascade and re-run helm upgrade:

```bash
kubectl delete statefulset docbrain-opensearch --cascade=orphan
helm upgrade docbrain ./helm/docbrain -f my-values.yaml
```

### Helm stuck in `pending-install`

```bash
kubectl delete secret sh.helm.release.v1.docbrain.v1
helm install docbrain ./helm/docbrain -f my-values.yaml
```
