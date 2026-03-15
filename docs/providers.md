# LLM & Embedding Provider Setup

DocBrain supports multiple LLM and embedding providers. Choose based on your requirements for quality, cost, latency, and data privacy.

## LLM Providers

### Anthropic (Recommended)

Best quality for documentation Q&A. Supports extended thinking for complex queries.

```env
LLM_PROVIDER=anthropic
ANTHROPIC_API_KEY=sk-ant-...
LLM_MODEL_ID=claude-sonnet-4-5-20250929
```

**Models**: `claude-sonnet-4-5-20250929` (recommended), `claude-opus-4-20250514`

### OpenAI

Widely available, good quality.

```env
LLM_PROVIDER=openai
OPENAI_API_KEY=sk-...
LLM_MODEL_ID=gpt-4o
```

**Models**: `gpt-4o` (recommended), `gpt-4o-mini` (faster/cheaper)

### Ollama (Local)

100% local inference. No API keys, no data leaves your machine.

```env
LLM_PROVIDER=ollama
OLLAMA_BASE_URL=http://host.docker.internal:11434
LLM_MODEL_ID=llama3.1:70b
```

**Setup**:
```bash
ollama pull llama3.1:70b
ollama serve
```

#### Model Selection — Critical for Answer Quality

DocBrain's RAG pipeline relies on the LLM to stay strictly grounded in retrieved documents and follow structured formatting rules. **Small models (7B-8B) will hallucinate, fabricate facts not in the sources, and produce verbose repetitive answers.** Choose the largest model your hardware supports:

| Model | Params | RAM Required | Quality | Notes |
|-------|--------|-------------|---------|-------|
| `llama3.1:70b` | 70B | 48GB+ | **Good** | Closest to cloud model quality. Recommended for production local deployments. |
| `qwen2.5:32b` | 32B | 26GB+ | **Good** | Strong instruction follower, competitive with 70B on grounding tasks. |
| `mistral-small:22b` | 22B | 16GB+ | Decent | Good middle ground for moderate hardware. |
| `phi4:14b` | 14B | 12GB+ | Decent | Better instruction following than larger 8B models. |
| `llama3.1` (8B) | 8B | 8GB+ | **Poor** | Will hallucinate, pad answers, and ignore grounding rules. Only use for quick testing, not real workloads. |

> **Warning**: Using 7B-8B models (like `llama3.1`, `mistral:7b`, `gemma2`) for Q&A will produce unreliable answers. The model will invent facts, ignore source citations, and generate verbose filler. If your hardware can only run 8B models, use a cloud LLM provider (Anthropic, OpenAI, Bedrock) for Q&A and Ollama only for embeddings — this is a fully supported mixed configuration.

**Vision models** (for image extraction): `llava`, `llama3.2-vision`, `moondream`, `bakllava`. If your `LLM_MODEL_ID` is a text-only model, image extraction is automatically skipped — no errors, no configuration needed.

### AWS Bedrock

For AWS-native deployments. Uses the **AWS SDK default credential chain** — no hardcoded keys required in production.

```env
LLM_PROVIDER=bedrock
AWS_REGION=us-east-1
LLM_MODEL_ID=us.anthropic.claude-opus-4-20250514-v1:0
```

#### AWS Credential Resolution Order

DocBrain uses `aws_config::defaults().load()`, which resolves credentials in this order:

1. **Environment variables** — `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` (local dev, CI)
2. **Shared credentials file** — `~/.aws/credentials` / `aws sso login` (local dev)
3. **IRSA (EKS)** — IAM Roles for Service Accounts (recommended for Kubernetes)
4. **EC2 Instance Profile** — attached IAM role (recommended for EC2/ECS)
5. **ECS Task Role** — `AWS_CONTAINER_CREDENTIALS_RELATIVE_URI`

#### Production Best Practice: IRSA (no keys in env)

On EKS, use IRSA so pods authenticate via their ServiceAccount — no `AWS_ACCESS_KEY_ID` needed:

```bash
helm install docbrain ./helm/docbrain \
  --set llm.provider=bedrock \
  --set serviceAccount.create=true \
  --set "serviceAccount.annotations.eks\.amazonaws\.com/role-arn=arn:aws:iam::123456789:role/docbrain-bedrock"
```

The IAM role needs these permissions:

```json
{
  "Effect": "Allow",
  "Action": [
    "bedrock:InvokeModel",
    "bedrock:InvokeModelWithResponseStream"
  ],
  "Resource": "arn:aws:bedrock:*::foundation-model/*"
}
```

Both the server and ingest CronJob pods use the same ServiceAccount, so a single IRSA role covers both.

#### Local Development

For local dev / docker-compose, explicit keys or `~/.aws/credentials` are fine:

```env
AWS_ACCESS_KEY_ID=AKIA...
AWS_SECRET_ACCESS_KEY=...
AWS_REGION=us-east-1
```

## Embedding Providers

### OpenAI Embeddings

```env
EMBED_PROVIDER=openai
OPENAI_API_KEY=sk-...
EMBED_MODEL_ID=text-embedding-3-small
```

**Models**: `text-embedding-3-small` (1536d, recommended), `text-embedding-3-large` (3072d)

### Ollama Embeddings

```env
EMBED_PROVIDER=ollama
OLLAMA_BASE_URL=http://host.docker.internal:11434
EMBED_MODEL_ID=nomic-embed-text
```

**Setup**: `ollama pull nomic-embed-text`

**Models**: `nomic-embed-text` (768d, recommended), `mxbai-embed-large` (1024d)

### AWS Bedrock Embeddings

```env
EMBED_PROVIDER=bedrock
EMBED_MODEL_ID=cohere.embed-v4:0
```

## Mixing Providers

You can use different providers for LLM and embeddings. Common combinations:

| Use Case | LLM | Embeddings |
|----------|-----|------------|
| Best quality | Anthropic | OpenAI |
| Fully local | Ollama | Ollama |
| Cost-optimized | OpenAI (gpt-4o-mini) | OpenAI (text-embedding-3-small) |
| AWS native | Bedrock | Bedrock |

> **Important**: Changing the embedding provider/model may change vector dimensions. The server will refuse to start with a dimension mismatch error. Set `FORCE_REINDEX=true` to delete and recreate the indexes, then run ingest to re-embed all documents. See [configuration.md](configuration.md#switching-embedding-models) for details.

## Model Recommendations

Based on testing across DocBrain's core workloads — RAG retrieval, intent classification, freshness scoring, and Autopilot draft generation — here are the configurations that deliver the best results.

### Quick Reference

| Priority | LLM | Embeddings | Notes |
|----------|-----|------------|-------|
| **Best quality** | `claude-sonnet-4-5-20250929` (Anthropic) | `text-embedding-3-small` (OpenAI) | Top answer accuracy and citation quality |
| **Best fully local** | `llama3.1:70b` (Ollama) | `mxbai-embed-large` (Ollama) | No data leaves your machine; needs 48GB+ RAM |
| **Local / mid-range** | `qwen2.5:32b` or `mistral-small:22b` (Ollama) | `mxbai-embed-large` (Ollama) | 16-26GB RAM; good quality for most queries |
| **Local / low resource** | Cloud LLM (Anthropic/OpenAI) | `nomic-embed-text` (Ollama) | Use cloud for Q&A, Ollama for embeddings only. 8B models produce unreliable answers. |
| **Cost-optimized cloud** | `gpt-4o-mini` (OpenAI) | `text-embedding-3-small` (OpenAI) | Good for high-volume teams on a budget |
| **AWS-native** | Claude Sonnet via Bedrock | Cohere via Bedrock | IAM auth, no key management |

### What We Observed

**Anthropic Claude Sonnet 4.5** produced the most accurate answers on multi-hop questions and handled DocBrain's structured prompt format (context blocks + freshness metadata) without truncation issues. Extended thinking helped on ambiguous procedural queries.

**Ollama `llama3.1:70b`** was the strongest local option — retrieval quality and draft generation were close to cloud models for straightforward factual and procedural queries. `qwen2.5:32b` is a strong alternative if you're RAM-constrained. **The 8B variant (`llama3.1`) is not recommended** — it consistently hallucinated facts not present in source documents, produced verbose repetitive answers, and failed to follow grounding constraints. If your hardware only supports 8B models, use a cloud LLM provider for Q&A and Ollama only for embeddings.

**Embeddings matter more than you might expect.** `nomic-embed-text` (Ollama) performed well for semantic similarity but lagged on keyword-dense technical content (CLI flags, error codes). If you're on Ollama for LLM but have network access, using `text-embedding-3-small` for embeddings is a practical middle ground.

**`gpt-4o-mini`** is a solid cost/quality tradeoff for teams already on OpenAI — it handles most queries well but occasionally misses nuance on long context windows with many retrieved chunks.

### Recommended Starting Configuration

For most teams getting started:

```env
# LLM — best quality
LLM_PROVIDER=anthropic
ANTHROPIC_API_KEY=sk-ant-...
LLM_MODEL_ID=claude-sonnet-4-5-20250929

# Embeddings — fast and accurate
EMBED_PROVIDER=openai
OPENAI_API_KEY=sk-...
EMBED_MODEL_ID=text-embedding-3-small
```

For fully air-gapped / local deployments:

```env
# Both LLM and embeddings via Ollama
LLM_PROVIDER=ollama
OLLAMA_BASE_URL=http://host.docker.internal:11434
LLM_MODEL_ID=llama3.1:70b

EMBED_PROVIDER=ollama
EMBED_MODEL_ID=mxbai-embed-large
```

```bash
# Pull both models before starting
ollama pull llama3.1:70b
ollama pull mxbai-embed-large
```

> **Tip**: If you're using Ollama for a fully local setup and find answer quality lacking, try increasing `RAG_TOP_K` to `15` and `CHUNK_SIZE` to `2000`. Smaller local models benefit more from additional retrieved context than cloud models do.

> **Hardware-constrained?** If you can't run 30B+ models locally, use a **mixed configuration**: cloud LLM for Q&A (Anthropic or OpenAI) + Ollama for embeddings. This keeps embedding data local while getting cloud-grade answer quality. See [Mixing Providers](#mixing-providers).
