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
LLM_MODEL_ID=command-r:35b
```

**Setup**:
```bash
ollama pull command-r:35b
ollama serve
```

#### Tuning for 70B and other large models

- **Speed up "Understanding" and "Searching"**: If you use a large model (e.g. `command-r:35b`), intent classification and query rewriting also use it when `FAST_MODEL_ID` is unset, so those phases can be slow. Set `FAST_MODEL_ID` to a small model (e.g. `qwen2.5:7b`) so only the final answer uses the primary model; intent and rewrite stay fast:
  ```env
  LLM_MODEL_ID=command-r:35b
  FAST_MODEL_ID=qwen2.5:7b
  ```
- **"Error decoding response body" after 2–3 minutes**: The default HTTP timeout is 120 seconds. If the 70B model takes longer to generate the full response, the connection is cut and you get a decode error. Set `OLLAMA_TIMEOUT_SECS=300` (or `600`):
  ```env
  OLLAMA_TIMEOUT_SECS=300
  ```

#### Model Selection — Critical for Answer Quality

DocBrain's RAG pipeline relies on the LLM to stay strictly grounded in retrieved documents and follow structured formatting rules. **Only use models with strong instruction-following capabilities.** Models that ignore system prompts or default to training data instead of provided context will produce fabricated answers — even when the correct documents are retrieved.

> **Key insight**: Model size alone does not determine RAG quality. A 35B model purpose-built for RAG (like `command-r:35b`) will outperform a 70B general-purpose model that ignores grounding instructions. **Instruction-following ability is the single most important trait for a DocBrain LLM.**

| Model | Params | RAM Required | Quality | Notes |
|-------|--------|-------------|---------|-------|
| `command-r:35b` | 35B | 24GB+ | **Best** | **Recommended.** Purpose-built for RAG. Excellent instruction following — stays grounded in retrieved docs, cites sources, avoids fabrication. |
| `qwen2.5:32b` | 32B | 26GB+ | **Good** | Strong instruction follower, competitive on grounding tasks. Good alternative to command-r. |
| `llama3.1:70b` | 70B | 48GB+ | Decent | Large but weaker at following grounding instructions — can ignore retrieved docs and generate from training data. Use command-r:35b instead unless you specifically need 70B. |
| `mistral-small:22b` | 22B | 16GB+ | Decent | Good middle ground for moderate hardware. |
| `phi4:14b` | 14B | 12GB+ | Decent | Better instruction following than larger 8B models. |
| `qwen2.5:7b` | 7B | 8GB+ | Fast-only | **Recommended as `FAST_MODEL_ID`** for intent classification and query rewriting. Too small for final answer generation. |
| `llama3.1` (8B) | 8B | 8GB+ | **Poor** | Will hallucinate, pad answers, and ignore grounding rules. Only use for quick testing, not real workloads. |

> **Warning — instruction following matters more than size**: Using models that don't follow grounding instructions (including some large models like `llama3.1:70b`) can produce completely fabricated answers that look plausible but contain zero information from your actual documents. This is worse than a "not found" response because it erodes user trust from day one. Always verify that your chosen model respects the `DOCUMENTATION:` context block and cites sources.

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

### Google Gemini

Fast and capable Google AI models. Uses the Google AI API (no GCP account required — just a Gemini API key).

```env
LLM_PROVIDER=gemini
GEMINI_API_KEY=AIza...
LLM_MODEL_ID=gemini-2.5-flash
```

**Models**: `gemini-2.5-flash` (recommended, fast), `gemini-2.5-pro` (reasoning), `gemini-3.1-pro-preview` (latest)

### Vertex AI (GCP)

Run Google Gemini and third-party models (Llama, Mistral) on your GCP infrastructure. Authenticated via the GCP credential chain — no API key needed in production when using Workload Identity.

```env
LLM_PROVIDER=vertex_ai
VERTEX_PROJECT=my-gcp-project
VERTEX_REGION=us-central1
LLM_MODEL_ID=google/gemini-2.5-flash
```

**Models**: `google/gemini-2.5-flash`, `google/gemini-2.5-pro`, `google/gemini-3.1-pro-preview`, `meta/llama-3.3-70b-instruct-maas`

#### GCP Credential Resolution Order

DocBrain uses `gcp_auth` which resolves credentials in this order:

1. **`GOOGLE_APPLICATION_CREDENTIALS`** → path to a service account JSON key file (local dev, CI)
2. **Application Default Credentials** — `gcloud auth application-default login` (local dev)
3. **GKE Workload Identity** — pod-level IAM binding (recommended for Kubernetes)
4. **GCE Metadata Service** — auto-detected on Compute Engine, Cloud Run, Cloud Functions

#### Production Best Practice: Workload Identity (no keys in cluster)

On GKE, use Workload Identity so pods authenticate via their ServiceAccount:

```bash
# Create a GCP service account
gcloud iam service-accounts create docbrain-vertex \
  --project=my-gcp-project

# Grant Vertex AI User role
gcloud projects add-iam-policy-binding my-gcp-project \
  --member="serviceAccount:docbrain-vertex@my-gcp-project.iam.gserviceaccount.com" \
  --role="roles/aiplatform.user"

# Bind the GCP service account to the Kubernetes service account
gcloud iam service-accounts add-iam-policy-binding \
  docbrain-vertex@my-gcp-project.iam.gserviceaccount.com \
  --role roles/iam.workloadIdentityUser \
  --member "serviceAccount:my-gcp-project.svc.id.goog[docbrain/docbrain]"

helm install docbrain ./helm/docbrain \
  --set llm.provider=vertex_ai \
  --set llm.vertexProject=my-gcp-project \
  --set llm.vertexRegion=us-central1 \
  --set llm.modelId=google/gemini-2.5-flash \
  --set "serviceAccount.annotations.iam\.gke\.io/gcp-service-account=docbrain-vertex@my-gcp-project.iam.gserviceaccount.com"
```

#### Local Development

```env
GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account-key.json
```
Or authenticate with gcloud: `gcloud auth application-default login`

### DeepSeek

Cost-effective API with strong coding and reasoning capabilities.

```env
LLM_PROVIDER=deepseek
DEEPSEEK_API_KEY=sk-...
LLM_MODEL_ID=deepseek-chat
```

**Models**: `deepseek-chat` (DeepSeek V3, recommended), `deepseek-reasoner` (R1, extended reasoning)

### Groq

Extremely fast inference (LPU hardware). Best for latency-sensitive workloads.

```env
LLM_PROVIDER=groq
GROQ_API_KEY=gsk_...
LLM_MODEL_ID=llama-3.3-70b-versatile
```

**Models**: `llama-3.3-70b-versatile` (recommended), `llama-3.1-8b-instant` (for `FAST_MODEL_ID`), `mixtral-8x7b-32768`

### Mistral

European provider, strong multilingual support and competitive pricing.

```env
LLM_PROVIDER=mistral
MISTRAL_API_KEY=...
LLM_MODEL_ID=mistral-small-latest
```

**Models**: `mistral-small-latest` (recommended), `mistral-medium-latest`, `codestral-latest` (code)

### xAI (Grok)

```env
LLM_PROVIDER=xai
XAI_API_KEY=xai-...
LLM_MODEL_ID=grok-3
```

**Models**: `grok-3`, `grok-3-mini` (for `FAST_MODEL_ID`)

### OpenRouter

Single API key across 100+ models (OpenAI, Anthropic, Gemini, Llama, Mistral, and more). Useful for testing different models without managing multiple API keys.

```env
LLM_PROVIDER=openrouter
OPENROUTER_API_KEY=sk-or-...
LLM_MODEL_ID=anthropic/claude-sonnet-4-5
```

**Models**: Any model slug from [openrouter.ai/models](https://openrouter.ai/models) — e.g. `openai/gpt-4o`, `meta-llama/llama-3.3-70b-instruct`

### Together AI

Hosting for open-source models with competitive pricing.

```env
LLM_PROVIDER=together
TOGETHER_API_KEY=...
LLM_MODEL_ID=meta-llama/Llama-3.3-70B-Instruct-Turbo
```

### Azure OpenAI

OpenAI models behind your Azure subscription. Uses `api-key` auth with your Azure endpoint.

```env
LLM_PROVIDER=azure_openai
AZURE_OPENAI_API_KEY=...
AZURE_OPENAI_ENDPOINT=https://my-resource.openai.azure.com
LLM_MODEL_ID=gpt-4o                    # your deployment name
# AZURE_OPENAI_API_VERSION=2024-02-01  # default
```

The `LLM_MODEL_ID` must match your **deployment name** in Azure OpenAI Studio (not the underlying model name).

### Cohere

```env
LLM_PROVIDER=cohere
COHERE_API_KEY=...
LLM_MODEL_ID=command-r-plus
```

**Models**: `command-r-plus` (recommended, strong RAG), `command-r` (faster/cheaper)

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
| Cost-optimized | DeepSeek (`deepseek-chat`) | OpenAI (`text-embedding-3-small`) |
| Fast inference | Groq | OpenAI |
| AWS-native | Bedrock | Bedrock |
| GCP-native | Vertex AI | OpenAI or Bedrock |
| Enterprise Microsoft | Azure OpenAI | OpenAI |
| Multi-model testing | OpenRouter | OpenAI |

> **Important**: Changing the embedding provider/model may change vector dimensions. The server will refuse to start with a dimension mismatch error. Set `FORCE_REINDEX=true` to delete and recreate the indexes, then run ingest to re-embed all documents. See [configuration.md](configuration.md#switching-embedding-models) for details.

## Model Recommendations

Based on testing across DocBrain's core workloads — RAG retrieval, intent classification, freshness scoring, and Autopilot draft generation — here are the configurations that deliver the best results.

### Quick Reference

| Priority | LLM | Embeddings | Notes |
|----------|-----|------------|-------|
| **Best quality** | `claude-sonnet-4-5-20250929` (Anthropic) | `text-embedding-3-small` (OpenAI) | Top answer accuracy and citation quality |
| **Best fully local** | `command-r:35b` (Ollama) | `mxbai-embed-large` (Ollama) | No data leaves your machine; 24GB+ RAM. Purpose-built for RAG. |
| **Local / mid-range** | `qwen2.5:32b` or `mistral-small:22b` (Ollama) | `mxbai-embed-large` (Ollama) | 16-26GB RAM; good quality for most queries |
| **Local / low resource** | Cloud LLM (Anthropic/OpenAI) | `nomic-embed-text` (Ollama) | Use cloud for Q&A, Ollama for embeddings only. 8B models produce unreliable answers. |
| **Cost-optimized cloud** | `gpt-4o-mini` (OpenAI) | `text-embedding-3-small` (OpenAI) | Good for high-volume teams on a budget |
| **AWS-native** | Claude Sonnet via Bedrock | Cohere via Bedrock | IAM auth, no key management |

### What We Observed

**Anthropic Claude Sonnet 4.5** produced the most accurate answers on multi-hop questions and handled DocBrain's structured prompt format (context blocks + freshness metadata) without truncation issues. Extended thinking helped on ambiguous procedural queries.

**Ollama `command-r:35b`** is now the recommended local model. It is purpose-built for RAG workloads — it stays grounded in retrieved documents, cites sources accurately, and follows structured prompt instructions far better than general-purpose models of similar or larger size. `qwen2.5:32b` is a strong alternative. We previously recommended `llama3.1:70b`, but found it frequently defaults to training data instead of retrieved context, producing plausible-sounding but fabricated answers — a worse outcome than "not found" because it erodes user trust. For `FAST_MODEL_ID`, use `qwen2.5:7b` — it handles intent classification and query rewriting well without the hallucination risks of using a small model for final answer generation. **The 8B variant (`llama3.1`) is not recommended** — it consistently hallucinated facts not present in source documents, produced verbose repetitive answers, and failed to follow grounding constraints. If your hardware only supports 8B models, use a cloud LLM provider for Q&A and Ollama only for embeddings.

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
LLM_MODEL_ID=command-r:35b
FAST_MODEL_ID=qwen2.5:7b        # use 7B for intent/rewrite; only final answer uses primary model
OLLAMA_TIMEOUT_SECS=300          # increase for large models

EMBED_PROVIDER=ollama
EMBED_MODEL_ID=mxbai-embed-large
```

```bash
# Pull models before starting
ollama pull command-r:35b
ollama pull qwen2.5:7b     # for FAST_MODEL_ID
ollama pull mxbai-embed-large
```

> **Tip**: If you're using Ollama for a fully local setup and find answer quality lacking, try increasing `RAG_TOP_K` to `15` and `CHUNK_SIZE` to `2000`. Smaller local models benefit more from additional retrieved context than cloud models do.

> **Hardware-constrained?** If you can't run 30B+ models locally, use a **mixed configuration**: cloud LLM for Q&A (Anthropic or OpenAI) + Ollama for embeddings. This keeps embedding data local while getting cloud-grade answer quality. See [Mixing Providers](#mixing-providers).
