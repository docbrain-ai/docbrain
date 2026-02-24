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
