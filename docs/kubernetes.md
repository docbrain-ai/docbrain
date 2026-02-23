# Kubernetes Deployment

Deploy DocBrain to Kubernetes using the included Helm chart.

## Prerequisites

- Kubernetes 1.26+
- Helm 3.x
- `kubectl` configured for your cluster

## Quick Install

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=anthropic \
  --set llm.anthropicApiKey=sk-ant-... \
  --set embedding.provider=openai \
  --set embedding.openaiApiKey=sk-...
```

## Configuration

### Using External Databases

By default, the chart deploys PostgreSQL, OpenSearch, and Redis in-cluster. For production, point to managed services:

```bash
helm install docbrain ./helm/docbrain \
  --set postgresql.internal=false \
  --set postgresql.externalUrl="postgresql://user:pass@rds-host:5432/docbrain" \
  --set opensearch.internal=false \
  --set opensearch.externalUrl="https://opensearch-host:9200" \
  --set redis.internal=false \
  --set redis.externalUrl="redis://elasticache-host:6379"
```

### Using Existing Secrets

```bash
kubectl create secret generic docbrain-secrets \
  --from-literal=ANTHROPIC_API_KEY=sk-ant-... \
  --from-literal=POSTGRES_PASSWORD=...

helm install docbrain ./helm/docbrain \
  --set existingSecret=docbrain-secrets
```

### Enabling Ingress

```bash
helm install docbrain ./helm/docbrain \
  --set ingress.enabled=true \
  --set ingress.host=docbrain.yourcompany.com \
  --set ingress.tls=true \
  --set ingress.tlsSecretName=docbrain-tls
```

### Ollama Mode (Local LLM)

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=ollama \
  --set llm.ollamaBaseUrl=http://ollama-service:11434 \
  --set llm.modelId=llama3.1 \
  --set embedding.provider=ollama \
  --set embedding.modelId=nomic-embed-text
```

## Values Reference

See `helm/docbrain/values.yaml` for the complete list of configurable values.

## Monitoring

The server exposes a `/health` endpoint used by Kubernetes probes. For deeper monitoring:

- **Metrics**: Application logs include structured timing information
- **Traces**: Each RAG pipeline phase is logged with duration
- **Alerts**: Monitor the CronJob for ingestion failures

## Scaling

- **API Server**: Increase `server.replicas` for horizontal scaling. Sessions are stored in Redis, so any replica can serve any request.
- **OpenSearch**: For large document sets, use a managed OpenSearch cluster with multiple data nodes.
- **Ingestion**: The CronJob runs as a single instance (`concurrencyPolicy: Forbid`).
