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

100% local inference. No API keys, no data leaves your machine. Requires a machine with sufficient RAM (16GB+ recommended).

```env
LLM_PROVIDER=ollama
OLLAMA_BASE_URL=http://host.docker.internal:11434
LLM_MODEL_ID=llama3.1
```

**Setup**:
```bash
ollama pull llama3.1
ollama serve
```

**Models**: `llama3.1` (recommended, 8B), `llama3.1:70b` (better quality, needs 48GB+ RAM)

**Vision models** (for image extraction): `llava`, `llama3.2-vision`, `moondream`, `bakllava`. If your `LLM_MODEL_ID` is a text-only model, image extraction is automatically skipped — no errors, no configuration needed.

### AWS Bedrock

For AWS-native deployments. Uses IAM for authentication.

```env
LLM_PROVIDER=bedrock
AWS_REGION=us-east-1
AWS_ACCESS_KEY_ID=...
AWS_SECRET_ACCESS_KEY=...
LLM_MODEL_ID=us.anthropic.claude-opus-4-20250514-v1:0
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

> **Important**: Changing the embedding provider after initial ingestion requires re-indexing all documents, as embedding dimensions may differ between providers.

## Model Recommendations

Based on testing across DocBrain's core workloads — RAG retrieval, intent classification, freshness scoring, and Autopilot draft generation — here are the configurations that deliver the best results.

### Quick Reference

| Priority | LLM | Embeddings | Notes |
|----------|-----|------------|-------|
| **Best quality** | `claude-sonnet-4-5-20250929` (Anthropic) | `text-embedding-3-small` (OpenAI) | Top answer accuracy and citation quality |
| **Best fully local** | `llama3.1:70b` (Ollama) | `mxbai-embed-large` (Ollama) | No data leaves your machine; needs 48GB+ RAM |
| **Local / low resource** | `llama3.1` 8B (Ollama) | `nomic-embed-text` (Ollama) | Runs on 16GB RAM; quality drops on complex queries |
| **Cost-optimized cloud** | `gpt-4o-mini` (OpenAI) | `text-embedding-3-small` (OpenAI) | Good for high-volume teams on a budget |
| **AWS-native** | Claude Sonnet via Bedrock | Cohere via Bedrock | IAM auth, no key management |

### What We Observed

**Anthropic Claude Sonnet 4.5** produced the most accurate answers on multi-hop questions and handled DocBrain's structured prompt format (context blocks + freshness metadata) without truncation issues. Extended thinking helped on ambiguous procedural queries.

**Ollama `llama3.1:70b`** was the strongest local option — retrieval quality and draft generation were close to cloud models for straightforward factual and procedural queries. The 8B variant is viable for teams with strict data-residency requirements but expect degraded performance on comparative and troubleshooting intents.

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
