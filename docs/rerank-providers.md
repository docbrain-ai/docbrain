# Rerank Providers

DocBrain ships with plug-and-play support for every major hosted rerank API plus a local fallback. Reranking is stage 3 of the retrieval pipeline — it takes the candidate pool from stage 2 and rescores each `(query, chunk)` pair with a cross-encoder, producing calibrated scores in `[0, 1]` that drive the grounding floors.

## Why it matters

Vector search and BM25 both return unbounded scores that are **not comparable across queries**. Rerank scores are. That's what lets DocBrain set a single `min_relevance_score` threshold and have it behave the same way for every query and every corpus.

Rerank quality directly controls:

- **Answer grounding** — low-confidence candidates get filtered out before they reach the LLM.
- **Citation accuracy** — top-ranked chunks are what the LLM cites.
- **False-negative rate** — how often "I don't know" is actually "I have it, but didn't surface it."

## The dialect model

Hosted rerank APIs converged on the same request/response shape: `{query, documents, top_n, model}` in, `[{index, score}]` out. Only the JSON field names drift. DocBrain captures this as a **dialect** — a descriptor that tells one shared HTTP client how each API names its fields, authenticates, and packages its response.

That means:

- **Built-in dialects** (Cohere, Voyage, Jina, Mixedbread, Pinecone) are single match arms. Zero per-provider code.
- **New providers** can be added at runtime via `provider: "custom"` without recompiling DocBrain.
- **Bedrock** stays on a separate AWS SDK path because the wire format is the same but the transport isn't.
- **Ollama** stays on a separate path because it has no native rerank endpoint (see below).

## Provider matrix

| Provider | Dialect | Default model | Auth | Notes |
|---|---|---|---|---|
| Bedrock | AWS SDK | `cohere.rerank-v3-5:0` | IAM (default credential chain) | Runs Cohere Rerank v3.5 via AWS Bedrock |
| Cohere | HTTP | `rerank-v3.5` | `Bearer ${COHERE_RERANK_API_KEY}` | Direct Cohere API |
| Voyage | HTTP | `rerank-2` | `Bearer ${VOYAGE_API_KEY}` | Uses `top_k` + `data` (vs Cohere's `top_n` + `results`) |
| Jina | HTTP | `jina-reranker-v2-base-multilingual` | `Bearer ${JINA_API_KEY}` | Multilingual strength |
| Mixedbread | HTTP | `mixedbread-ai/mxbai-rerank-large-v1` | `Bearer ${MIXEDBREAD_API_KEY}` | Uses `input` instead of `documents` |
| Pinecone | HTTP | `bge-reranker-v2-m3` | `Api-Key: ${PINECONE_API_KEY}` | Custom header, not Bearer |
| Ollama | Local embeddings | `nomic-embed-text` | none | Bi-encoder approximation — local, no key |
| Custom | HTTP | *(operator supplies)* | operator config | Any Cohere-family API without a rebuild |

## Quick start — built-in providers

Pick a provider, set the corresponding API key in the environment, and set `RAG_RERANK_PROVIDER`. DocBrain picks up the default model, request shape, and auth style automatically.

### Cohere

```bash
export RAG_RERANK_PROVIDER=cohere
export COHERE_RERANK_API_KEY=co-xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
# Optional: override the default model
# export RAG_RERANK_MODEL_ID=rerank-english-v3.0
```

### Voyage AI

```bash
export RAG_RERANK_PROVIDER=voyage
export VOYAGE_API_KEY=pa-xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
# export RAG_RERANK_MODEL_ID=rerank-2-lite
```

### Jina AI

```bash
export RAG_RERANK_PROVIDER=jina
export JINA_API_KEY=jina_xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
# export RAG_RERANK_MODEL_ID=jina-reranker-v2-base-multilingual
```

### Mixedbread

```bash
export RAG_RERANK_PROVIDER=mixedbread
export MIXEDBREAD_API_KEY=mxb_xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

### Pinecone Inference

```bash
export RAG_RERANK_PROVIDER=pinecone
export PINECONE_API_KEY=pcsk_xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
# export RAG_RERANK_MODEL_ID=bge-reranker-v2-m3
```

### Bedrock (Cohere via AWS)

```bash
export RAG_RERANK_PROVIDER=bedrock
# Credentials come from the default AWS credential chain (IAM role,
# AWS_PROFILE, or AWS_ACCESS_KEY_ID + AWS_SECRET_ACCESS_KEY).
# Must build DocBrain with --features bedrock.
```

### Ollama (local, no key)

```bash
export RAG_RERANK_PROVIDER=ollama
# Optional:
# export RAG_RERANK_OLLAMA_BASE_URL=http://localhost:11434
# export RAG_RERANK_MODEL_ID=nomic-embed-text
```

**Honest caveat:** Ollama has no first-class rerank endpoint. DocBrain approximates rerank by computing cosine similarity between query and document embeddings from any Ollama embedding model. This is a bi-encoder, not a true cross-encoder — quality is meaningfully lower than hosted providers. Use it for local development, air-gapped deployments, or as a baseline; reach for a hosted provider when quality matters.

For true cross-encoder quality locally, run `bge-reranker` or `mxbai-rerank` through a small HTTP server and use the `custom` provider below.

## Adding a new provider in 2 minutes — the `custom` dialect

DocBrain's reranker is plug-and-play: if your target API follows the Cohere-family convention (POST JSON with `query` + documents + top-N, response has a results array with index + score), you can wire it with environment variables alone.

### Minimal example — a Cohere-compatible API

```bash
export RAG_RERANK_PROVIDER=custom
export RAG_RERANK_CUSTOM_BASE_URL=https://rerank.mycorp.internal/v1/rerank
export RAG_RERANK_CUSTOM_API_KEY_ENV=MYCORP_RERANK_KEY
export RAG_RERANK_MODEL_ID=mycorp-rerank-v1
export MYCORP_RERANK_KEY=secret-xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

That's it. Restart `docbrain-server`. DocBrain sends:

```http
POST https://rerank.mycorp.internal/v1/rerank
Authorization: Bearer secret-xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
Content-Type: application/json

{
  "query": "how is sp-brain deployed",
  "documents": ["chunk 1 text...", "chunk 2 text...", "..."],
  "model": "mycorp-rerank-v1"
}
```

and expects:

```json
{
  "results": [
    { "index": 2, "relevance_score": 0.93 },
    { "index": 0, "relevance_score": 0.71 }
  ]
}
```

### When your API uses different field names

Every JSON key is overridable. For example, an API that calls its documents `passages`, uses `top_k` instead of `top_n`, returns results in `data`, and scores them as `score`:

```bash
export RAG_RERANK_PROVIDER=custom
export RAG_RERANK_CUSTOM_BASE_URL=https://api.acme.test/rerank
export RAG_RERANK_CUSTOM_API_KEY_ENV=ACME_KEY
export RAG_RERANK_MODEL_ID=acme-rerank-v2
export RAG_RERANK_CUSTOM_DOCUMENTS_FIELD=passages
export RAG_RERANK_CUSTOM_TOP_N_FIELD=top_k
export RAG_RERANK_CUSTOM_RESULTS_FIELD=data
export RAG_RERANK_CUSTOM_SCORE_FIELD=score
export ACME_KEY=acme-xxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

### When your API uses a custom auth header

Pinecone-style `Api-Key: <key>` instead of `Authorization: Bearer <key>`:

```bash
export RAG_RERANK_CUSTOM_AUTH_STYLE=custom_header
export RAG_RERANK_CUSTOM_AUTH_HEADER_NAME=Api-Key
```

### Via config.yaml instead of env vars

All custom fields have a matching YAML field under `rerank:`. Environment variables take precedence, so you can pin defaults in YAML and override per-environment without editing files:

```yaml
rerank:
  provider: custom
  model_id: acme-rerank-v2
  custom_base_url: https://api.acme.test/rerank
  custom_api_key_env: ACME_KEY
  custom_documents_field: passages
  custom_top_n_field: top_k
  custom_results_field: data
  custom_score_field: score
```

## Tuning

### Reranker-specific knobs

| Knob | Tradeoff |
|---|---|
| `RAG_RERANK_TOP_N` | Higher = better recall at the top, linear cost/latency growth. Should match `rag.candidate_pool_size`. |
| `RAG_RERANK_BATCH_SIZE` | Lower = more HTTP calls but smaller per-call latency spikes. Clamped to `[1, 1000]`. |
| `RAG_RERANK_TIMEOUT_SECS` | Tight — rerank is on the critical path of every query. Failure falls back to the RRF-only ranking from stage 2. |

### Tuning the grounding floors

The reranker's score only matters insofar as it gates the four **grounding floors** in `rag.*`. These are the biggest quality lever in the whole pipeline and the most common source of "why is this irrelevant doc cited?" complaints. See the detailed [Grounding floors — what lowering actually costs](configuration.md#grounding-floors--what-lowering-actually-costs) section in the main configuration reference for the full story.

**TL;DR — the recommended defaults for a cross-encoder reranker:**

| Floor | Recommended default | Meaning | What lowering costs |
|---|---|---|---|
| `rag.min_relevance_score` | `0.40` | Chunks below this never reach the LLM | **Hallucination risk** — the LLM sees weaker evidence and writes confident answers from chunks that only tangentially match |
| `rag.display_floor` | `0.50` | Chunks below this are never shown as citations | **User trust** — tangentially-related docs appear in the sources list and erode credibility |
| `rag.confidence_gate` | `0.40` | Below this, sources are hidden entirely (answer shown as "general knowledge") | Sources render on low-confidence answers that may mislead users |
| `rag.strong_answer_floor` | `0.55` | Below this, the answer carries a "low confidence" disclaimer | The UI stops warning users about borderline matches |

**Calibration note.** A cross-encoder's `[0, 1]` score is **not** a percentage. For Cohere Rerank v3.5, Voyage rerank-2, and similar models, `> 0.70` means "directly answers the question", `0.50–0.70` is "strong supporting evidence", `0.40–0.50` is "topically related", `0.30–0.40` is "shares keywords but usually noise". The defaults draw the lines at "topically related" for retrieval and "strong evidence" for citation display.

**If `rerank.provider = "none"`:** these floors gate on raw BM25/vector scores which are **not** calibrated to `[0, 1]`. Set all four to `0.0` in that mode and rely on `top_k` to bound results. A real reranker is what makes these floors work at all — which is why the plug-and-play providers in this doc exist.

**Debugging a noisy citation.** Run `docbrain trace-query "your question"` and look at the `rerank` stage log line. If the offending citation is scoring `0.30–0.45`, raising `display_floor` will fix it. If it's scoring `> 0.50`, the reranker genuinely thinks it's relevant and the problem is upstream (candidate pool, query decomposition, or title-enrichment metadata leak).

## Operational notes

- **Fail-loud:** if the selected provider is missing credentials or (for `custom`) a required dialect field, DocBrain fails at startup with a message naming both the config field and its env var. There is no silent fallback to `none`.
- **Retries:** transient errors (`429`, `500–504`, timeouts, connect failures) retry up to 3 times with exponential backoff — same policy as the LLM and embedder HTTP paths.
- **Batching:** pools larger than `batch_size` split into multiple requests. Every input id appears in the output even if the upstream drops some (fail-loud on data, not on trait contract).
- **Score calibration:** scores are clamped to `[0, 1]` before they leave the provider. Downstream floors don't care which provider produced them.
- **Secrets never in config.yaml:** the `custom_api_key_env` field names an *env var*; DocBrain reads the secret from the environment at startup. This keeps YAML safe to commit.

## When to pick which provider

- **You're on AWS and already have Bedrock access** → `bedrock`. Zero new credentials, same IAM story as the LLM and embedder paths.
- **You want the highest-quality managed rerank** → `cohere` (or Bedrock's Cohere) and Voyage are the current leaders.
- **You need multilingual** → `jina` (`jina-reranker-v2-base-multilingual`) is purpose-built for it.
- **You're already paying for Pinecone vector search** → `pinecone` keeps everything in one vendor.
- **Local development, air-gapped, or no budget** → `ollama`. Accept the quality tradeoff.
- **Your cloud has a private rerank service or an OEM deal** → `custom`. Wire it with env vars.
- **You want a true local cross-encoder** → serve `bge-reranker` or `mxbai-rerank` behind a small HTTP wrapper and use `custom`.
