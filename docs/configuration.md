# Configuration Reference

## How Configuration Works

DocBrain uses a **config-first architecture** with a layered YAML + environment variable system. Understanding this prevents confusion about why a value isn't taking effect.

### Loading Order (later = higher priority)

```
config/default.yaml         ← committed to repo — all non-secret defaults
config/{APP_ENV}.yaml       ← environment-specific overrides (development | production)
config/local.yaml           ← gitignored — your secrets and local overrides
Environment variables / .env ← always win — highest priority
```

Set `APP_ENV=production` for the production profile (this is the default in the Docker image). The server defaults to `APP_ENV=development` when running locally without Docker.

### What Goes Where

| Type | Where to put it |
|---|---|
| Infrastructure secrets (DB URL, LLM API keys, Redis, OpenSearch) | `.env` or environment variables |
| Ingest source credentials (Confluence token, GitHub token, Slack token, Jira token) | `config/local.yaml` (gitignored) |
| Deployment-specific values (URLs, ports, CORS origins) | `.env` or environment variables |
| Tuning (thresholds, intervals, cache TTLs) | `config/local.yaml` or env vars |
| Team-wide defaults you want committed | `config/default.yaml` (no secrets!) |

**The key distinction:** `.env` is for infrastructure secrets that the runtime environment must inject (container orchestration, CI/CD, secrets managers). `config/local.yaml` is for user-managed source credentials and personal overrides — it's gitignored so it never gets committed, but it lives alongside the project where you can edit it easily.

### Example `config/local.yaml`

```yaml
# config/local.yaml — never committed (gitignored)
# Configure ingest sources and personal overrides here.

confluence:
  base_url: https://acme.atlassian.net/wiki
  user_email: you@acme.com
  api_token: ATATT3x...
  space_keys: DOCS,ENG

sources:
  github:
    token: ghp_...
    pull_requests:
      repos:
        - acme/platform
        - acme/docs
      lookback_days: 180
  jira:
    base_url: https://acme.atlassian.net
    user_email: you@acme.com
    api_token: ATATT3x...
    projects:
      - ENG
      - PLAT

# Local tuning overrides (optional)
autopilot:
  enabled: true
  cluster_threshold: 0.78

rag:
  cache_ttl_hours: 1
```

### YAML Config Structure

Every YAML value supports `${ENV_VAR}` and `${ENV_VAR:-default}` substitution:

```yaml
database:
  url: "${DATABASE_URL}"     # required — must come from env
  max_connections: "${DB_MAX_CONNECTIONS:-10}"
```

### Custom Config Directory

```bash
# Mount a ConfigMap in Kubernetes
DOCBRAIN_CONFIG_DIR=/etc/docbrain docbrain-server

# Or pass as CLI argument
docbrain-server --config-dir /etc/docbrain
```

---

All configuration is also available via environment variables, set in `.env` for Docker Compose or via ConfigMap/Secret for Kubernetes. **Environment variables always override YAML values.**

## Infrastructure

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | — | PostgreSQL connection string |
| `OPENSEARCH_URL` | `http://localhost:9200` | OpenSearch endpoint |
| `REDIS_URL` | `redis://localhost:6379` | Redis connection string |
| `SERVER_PORT` | `3000` | API server listen port |
| `SERVER_BIND` | `0.0.0.0` | API server bind address |
| `LOG_LEVEL` | `info` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |
| `DB_MAX_CONNECTIONS` | `10` | Maximum PostgreSQL connection pool size |
| `DB_CONNECT_TIMEOUT_SECS` | `10` | Timeout (seconds) for initial PostgreSQL connection |
| `DB_ACQUIRE_TIMEOUT_SECS` | `10` | Timeout (seconds) to acquire a connection from the pool |
| `DB_IDLE_TIMEOUT_SECS` | `300` | Idle connection lifetime (seconds) before cleanup |

## LLM Provider

| Variable | Default | Description |
|----------|---------|-------------|
| `LLM_PROVIDER` | `bedrock` | Provider: `bedrock`, `anthropic`, `openai`, `ollama`, `groq`, `openrouter`, `together`, `deepseek`, `mistral`, `xai`, `gemini`, `azure_openai`, `vertex_ai`, `cohere` |
| `LLM_MODEL_ID` | varies | Model identifier (provider-specific) |
| `FAST_MODEL_ID` | — | Fast/cheap model for background side-calls: intent classification, query rewriting, entity extraction. Falls back to `LLM_MODEL_ID` if not set. Recommended: Haiku (Bedrock/Anthropic), `gpt-4o-mini` (OpenAI), `qwen2.5:7b` (Ollama). Alias: `HAIKU_MODEL_ID` (deprecated). |
| `INGEST_LLM_MODEL_ID` | — | Model used **during ingest only** for image extraction. Falls back to `LLM_MODEL_ID` if not set. **Set this to a cheaper model** — image extraction fires for every page with images. Using Opus 4 with `LLM_THINKING_BUDGET` without this override will cause throttling errors during ingest. |
| `DRAFT_MODEL_ID` | — | Model used for **autopilot draft generation** (two-phase reasoning + writing). Falls back to `LLM_MODEL_ID` if not set. Use a high-capability model here — drafts benefit from stronger reasoning. |
| `DRAFT_LLM_PROVIDER` | — | Provider for draft generation. Falls back to `LLM_PROVIDER` if not set. Allows cross-provider drafting — e.g. use Gemini Flash for Q&A but Anthropic Claude for drafts. |
| `LLM_THINKING_BUDGET` | — | Extended thinking token budget (tokens). Unset or `0` = disabled. Only applies to the primary `LLM_MODEL_ID`, never to `FAST_MODEL_ID` or `INGEST_LLM_MODEL_ID`. |
| `ANTHROPIC_API_KEY` | — | API key (if `LLM_PROVIDER=anthropic`) |
| `OPENAI_API_KEY` | — | API key (if `LLM_PROVIDER=openai`) |
| `OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama server URL |
| `OLLAMA_TIMEOUT_SECS` | `120` | HTTP timeout in seconds for Ollama requests. Increase for large/slow models (e.g. 70B) to avoid "error decoding response body" when the model takes longer than 2 minutes. Example: `300` or `600`. Allowed range: 60–900. |
| `OLLAMA_TLS_VERIFY` | `false` | Set to `true` to enforce TLS certificate validation for Ollama |
| `OLLAMA_VISION_ENABLED` | `true` | Set to `false` if your Ollama model doesn't support vision (skips image calls) |
| `AWS_REGION` | — | AWS region for Bedrock (e.g. `us-east-1`) |
| `AWS_ACCESS_KEY_ID` | — | AWS access key (optional — see credential chain below) |
| `AWS_SECRET_ACCESS_KEY` | — | AWS secret key (optional — see credential chain below) |
| `GROQ_API_KEY` | — | API key (if `LLM_PROVIDER=groq`) |
| `OPENROUTER_API_KEY` | — | API key (if `LLM_PROVIDER=openrouter`) |
| `TOGETHER_API_KEY` | — | API key (if `LLM_PROVIDER=together`) |
| `DEEPSEEK_API_KEY` | — | API key (if `LLM_PROVIDER=deepseek`) |
| `MISTRAL_API_KEY` | — | API key (if `LLM_PROVIDER=mistral`) |
| `XAI_API_KEY` | — | API key (if `LLM_PROVIDER=xai`) |
| `GEMINI_API_KEY` | — | API key (if `LLM_PROVIDER=gemini`) |
| `AZURE_OPENAI_API_KEY` | — | API key (if `LLM_PROVIDER=azure_openai`) |
| `AZURE_OPENAI_ENDPOINT` | — | Resource endpoint (if `LLM_PROVIDER=azure_openai`). e.g. `https://my-resource.openai.azure.com` |
| `AZURE_OPENAI_API_VERSION` | `2024-02-01` | API version (if `LLM_PROVIDER=azure_openai`) |
| `VERTEX_PROJECT` | — | GCP project ID (if `LLM_PROVIDER=vertex_ai`). Required. |
| `VERTEX_REGION` | `us-central1` | GCP region (if `LLM_PROVIDER=vertex_ai`) |
| `COHERE_API_KEY` | — | API key (if `LLM_PROVIDER=cohere`) |

> **AWS Credential Chain**: Bedrock uses the AWS SDK default credential chain: env vars → `~/.aws/credentials` → IRSA (EKS) → EC2 Instance Profile → ECS Task Role. In production, use IRSA or instance profiles — no keys in env. Set `serviceAccount.create=true` and `serviceAccount.annotations.eks.amazonaws.com/role-arn` in Helm. The IAM role needs `bedrock:InvokeModel` and `bedrock:InvokeModelWithResponseStream` permissions. See [providers.md](providers.md#aws-bedrock) for full setup details.

> **GCP Credential Chain**: Vertex AI uses `gcp_auth` which resolves credentials in this order: `GOOGLE_APPLICATION_CREDENTIALS` (service account key file) → Application Default Credentials (`gcloud auth application-default login`) → GKE Workload Identity → GCE/Cloud Run metadata service. In production on GKE, use Workload Identity — no keys needed in the cluster. See [providers.md](providers.md#vertex-ai-gcp) for Workload Identity setup details.

### Ollama: model selection and tuning

**Only use models with strong instruction-following capabilities.** DocBrain's RAG pipeline requires the LLM to stay strictly grounded in retrieved documents. Models that default to training data instead of provided context will produce fabricated answers. Recommended: `command-r:35b` (purpose-built for RAG). See [providers.md](providers.md#model-selection--critical-for-answer-quality) for the full model comparison table.

- **Recommended config**: `LLM_MODEL_ID=command-r:35b` and `FAST_MODEL_ID=qwen2.5:7b`. The fast model handles intent classification and query rewriting; only the final answer uses the primary model.
- **"Error decoding response body" after 2–3 minutes**: The default HTTP timeout is 120 seconds. If the model takes longer to generate the full response, the connection is cut and you get a decode error. Set `OLLAMA_TIMEOUT_SECS=300` (or `600`) so the client waits long enough.

## Embedding Provider

Set `EMBED_PROVIDER` to choose your embedding model. One of: `openai`, `bedrock`, `ollama`.

| Variable | Default | Description |
|----------|---------|-------------|
| `EMBED_PROVIDER` | `bedrock` | Provider: `bedrock`, `openai`, `ollama` |
| `EMBED_MODEL_ID` | varies | Embedding model identifier (e.g. `text-embedding-3-small`, `cohere.embed-v4:0`) |

### Switching Embedding Models

When you change `EMBED_PROVIDER` or `EMBED_MODEL_ID` to a model with different vector dimensions (e.g. Bedrock Cohere/1024 → Ollama nomic-embed-text/768), the server will **refuse to start** with a clear error:

```
Embedding dimension mismatch on index 'docbrain-chunks': existing=1024, required=768.
```

To migrate:

1. Set `FORCE_REINDEX=true` in your environment
2. Restart the server and run ingest — the old indexes are deleted and recreated
3. Remove `FORCE_REINDEX` after the migration completes

| Variable | Default | Description |
|----------|---------|-------------|
| `FORCE_REINDEX` | `false` | Delete and recreate OpenSearch indexes when embedding dimensions change. Set once during migration, then remove. |

## Retrieval Pipeline

DocBrain runs queries through a five-stage retrieval pipeline when a
reranker is configured:

1. **Query understanding** — rewrites + entity → space mapping
2. **Candidate generation** — parallel retrievers (BM25, vector,
   entity-exact, freshness, procedural, semantic) fused with
   Reciprocal Rank Fusion (RRF)
3. **Semantic reranking** — a cross-encoder (e.g. Cohere Rerank on
   Bedrock) scores every (query, candidate) pair on a calibrated
   `[0.0, 1.0]` scale
4. **Diversity + coverage** — per-source and per-document caps so one
   dominant source can't crowd out the LLM's context window
5. **Grounding floor** — chunks below a configurable relevance floor
   are dropped before the LLM sees them, preventing confident
   hallucination on noise

### Why it matters

Without a reranker, BM25 scoring systematically buries small specialised
sources under corpus-dominant ones: a single captured PR with 11 chunks
is structurally out-ranked by a 4000-page Confluence space that happens
to mention the same keywords. The cross-encoder reranker scores each
`(query, chunk)` pair directly, independent of corpus size, so a
precise answer in a small source can outrank a tangentially relevant
chunk in a huge one.

**The pipeline is opt-in.** Set `rerank.provider = "none"` (the default)
and DocBrain runs the legacy single-hybrid-search path with
byte-identical behaviour to before the feature existed. Set it to any
configured provider to activate the five-stage pipeline. Rollback is a
single env var flip — no code change, no rebuild, no data migration.

### Reranker (`rerank.*`)

Stage 3 of retrieval rescores the candidate pool with a cross-encoder, producing calibrated `[0, 1]` scores that drive the grounding floors. DocBrain supports every major hosted rerank API through a single dialect-driven HTTP client — adding a new provider is typically a config change, not a code change.

**Built-in providers:** `bedrock`, `cohere`, `voyage`, `jina`, `mixedbread`, `pinecone`, `ollama`. Plus `custom` for any other Cohere-family API without a rebuild.

```yaml
# config/local.yaml — any hosted provider, one env var away
rerank:
  provider: cohere                    # or: bedrock | voyage | jina | mixedbread | pinecone | ollama | custom
  # model_id: rerank-v3.5             # provider default applies when unset
  top_n: 200                          # candidates scored per query
  batch_size: 100                     # docs per reranker call
  timeout_secs: 10                    # per-call timeout
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `rerank.provider` | `RAG_RERANK_PROVIDER` | `none` | `none` \| `bedrock` \| `cohere` \| `voyage` \| `jina` \| `mixedbread` \| `pinecone` \| `ollama` \| `custom` |
| `rerank.model_id` | `RAG_RERANK_MODEL_ID` | varies | Provider-specific model. Built-in defaults: Bedrock `cohere.rerank-v3-5:0`, Cohere `rerank-v3.5`, Voyage `rerank-2`, Jina `jina-reranker-v2-base-multilingual`, Mixedbread `mxbai-rerank-large-v1`, Pinecone `bge-reranker-v2-m3`, Ollama `nomic-embed-text`. |
| `rerank.top_n` | `RAG_RERANK_TOP_N` | `200` | How many candidates the reranker scores per query. Should match `rag.candidate_pool_size`. |
| `rerank.batch_size` | `RAG_RERANK_BATCH_SIZE` | `100` | Docs per reranker API call. Larger pools split into multiple batches. Clamped to `[1, 1000]`. |
| `rerank.timeout_secs` | `RAG_RERANK_TIMEOUT_SECS` | `10` | Per-request timeout. Tight because the reranker sits on the hot path of every `/api/v1/ask` request. On failure the pipeline falls back to RRF-only ranking. |
| `rerank.cohere_api_key` | `COHERE_RERANK_API_KEY` | — | Required when `provider = "cohere"`. |
| `rerank.voyage_api_key` | `VOYAGE_API_KEY` | — | Required when `provider = "voyage"`. |
| `rerank.jina_api_key` | `JINA_API_KEY` | — | Required when `provider = "jina"`. |
| `rerank.mixedbread_api_key` | `MIXEDBREAD_API_KEY` | — | Required when `provider = "mixedbread"`. |
| `rerank.pinecone_api_key` | `PINECONE_API_KEY` | — | Required when `provider = "pinecone"`. Uses `Api-Key` header, not Bearer. |
| `rerank.ollama_base_url` | `RAG_RERANK_OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama endpoint for local reranking. Ollama is a bi-encoder approximation — see notes below. |

#### Custom provider — plug-and-play for any rerank API

Set `provider = "custom"` and fill the fields below to wire a new rerank API without rebuilding DocBrain. Defaults match Cohere's request/response shape; override any JSON key that differs.

| Key | Env var | Required | Default | Description |
|-----|---------|----------|---------|-------------|
| `rerank.custom_base_url` | `RAG_RERANK_CUSTOM_BASE_URL` | ✅ | — | Full POST URL, e.g. `https://rerank.mycorp.internal/v1/rerank` |
| `rerank.custom_api_key_env` | `RAG_RERANK_CUSTOM_API_KEY_ENV` | ✅ | — | Name of another env var that holds the API key (the key is never persisted in config.yaml) |
| `rerank.model_id` | `RAG_RERANK_MODEL_ID` | ✅ | — | Model id to send in the request body |
| `rerank.custom_auth_style` | `RAG_RERANK_CUSTOM_AUTH_STYLE` |  | `bearer_token` | `bearer_token` or `custom_header` |
| `rerank.custom_auth_header_name` | `RAG_RERANK_CUSTOM_AUTH_HEADER_NAME` | only with `custom_header` | — | Header name, e.g. `Api-Key` |
| `rerank.custom_documents_field` | `RAG_RERANK_CUSTOM_DOCUMENTS_FIELD` |  | `documents` | Request JSON key for the documents array |
| `rerank.custom_top_n_field` | `RAG_RERANK_CUSTOM_TOP_N_FIELD` |  | `top_n` | Request JSON key for the top-N limit |
| `rerank.custom_results_field` | `RAG_RERANK_CUSTOM_RESULTS_FIELD` |  | `results` | Response JSON key for the results array |
| `rerank.custom_score_field` | `RAG_RERANK_CUSTOM_SCORE_FIELD` |  | `relevance_score` | Response JSON key for the score |

**See [rerank-providers.md](rerank-providers.md)** for the provider matrix, per-provider quick-starts, and the "add a new provider in 2 minutes" walkthrough.

**Ollama caveat**: Ollama has no first-class rerank endpoint. DocBrain approximates rerank by cosine-similarity over query + document embeddings from any Ollama embedding model — a **bi-encoder**, not a cross-encoder. Quality is meaningfully lower than hosted providers; it exists for local development and air-gapped deployments. For true cross-encoder quality locally, run `bge-reranker` or `mxbai-rerank` behind a small HTTP wrapper and use `provider: custom`.

**Fail-loud**: a missing API key or an incomplete `custom_*` block fails at server startup with a message naming both the config field and its env var. There is no silent fallback to `none`.

### Pipeline knobs (`rag.*`)

Every pipeline parameter is configurable — nothing is hardcoded. These
defaults are the canonical-paper / standard-practice values; tune them
only when you have query latency or quality data to justify a change.

```yaml
rag:
  cache_threshold: 0.95                # existing cache knob
  cache_ttl_hours: 24                   # existing cache knob
  top_k: 10                             # final chunks sent to the LLM
  bm25_boost: 1.0                       # BM25 vs vector weight in hybrid

  # New knobs for the five-stage pipeline:
  candidate_pool_size: 200              # pool size fed to reranker
  rrf_k: 60                             # RRF damping constant
  max_per_source: 3                     # per-source cap in final top_k
  max_per_document: 2                   # per-document cap in final top_k
  # Grounding floors — calibrated for a cross-encoder reranker.
  # See "Grounding floors" below for what each one does and what
  # lowering them actually costs you.
  min_relevance_score: 0.40             # retrieval floor
  display_floor: 0.50                   # display floor (user-visible citations)
  confidence_gate: 0.40                 # confidence gate (show-sources threshold)
  strong_answer_floor: 0.55             # high-confidence answer threshold
  freshness_window_days: 7              # freshness retriever window
  freshness_source_types:               # which source types count as "fresh"
    - github_capture
    - gitlab_capture
    - slack_capture
    - ms_teams_capture
  entity_cache_ttl_secs: 300            # entity → space cache TTL
  max_rewrites: 2                       # query rewrites per ask
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `rag.candidate_pool_size` | `RAG_CANDIDATE_POOL_SIZE` | `200` | How many candidates the candidate generator produces for the reranker. Larger = better recall, more reranker cost. |
| `rag.rrf_k` | `RAG_RRF_K` | `60` | Reciprocal Rank Fusion damping constant. 60 is the canonical paper default. Larger = more democratic across retrievers; smaller = concentrates weight at top ranks. |
| `rag.max_per_source` | `RAG_MAX_PER_SOURCE` | `3` | Max chunks from any single source in the final top-k. Prevents a dominant source from monopolising the LLM context. Set to `top_k` to disable. |
| `rag.max_per_document` | `RAG_MAX_PER_DOCUMENT` | `2` | Max chunks from any single document in the final top-k. Prevents one long document from crowding out other relevant docs. Set to `top_k` to disable. |
| `rag.min_relevance_score` | `RAG_MIN_RELEVANCE_SCORE` | `0.40` | **Retrieval floor** — reranker score required to survive diversity selection and reach the LLM. Chunks below this are dropped before the LLM sees them, even if it means returning fewer than `top_k` results. **Lowering sends weaker evidence into the prompt, which raises hallucination risk** — the LLM will try to answer from chunks that only tangentially match. Raising forces more "insufficient information" answers. Set to `0.0` to disable (required when `rerank.provider = "none"`, because raw BM25/vector scores are not calibrated to [0,1]). |
| `rag.display_floor` | `RAG_DISPLAY_FLOOR` | `0.50` | **Display floor** — reranker score required for a chunk to appear in the `sources` array attached to the answer. Must be `>= min_relevance_score`. The LLM may still have used a chunk to form its answer even if it is hidden here. **Lowering surfaces more citations per answer, but includes tangentially-related docs that erode user trust** — the main cause of "why is this GitHub PR cited, it has nothing to do with my question?" complaints. Raising narrows the visible citation set to only high-confidence matches. |
| `rag.confidence_gate` | `RAG_CONFIDENCE_GATE` | `0.40` | **Confidence gate** — minimum composite confidence score required to show any sources at all. When confidence is below this, DocBrain emits the answer with a "based on general knowledge" framing and no citations, instead of citing weak evidence. **Lowering shows sources on lower-confidence answers** (useful when operators want to see what the retriever found, even when it wasn't enough). **Raising forces the UI to go source-less more often**, which is safer for end users but hides the retriever's partial matches from debugging. |
| `rag.strong_answer_floor` | `RAG_STRONG_ANSWER_FLOOR` | `0.55` | **Strong-answer floor** — top-1 reranker score required before the answer is emitted without a "low confidence" disclaimer. Below this threshold the answer carries a visible uncertainty warning; below `min_relevance_score` the query short-circuits to "insufficient information" without calling the LLM at all. **Lowering removes the uncertainty warning from more answers** (less noise in the UI, but users can't tell strong from borderline answers apart). Raising makes DocBrain more openly uncertain about marginal matches. |
| `rag.freshness_window_days` | `RAG_FRESHNESS_WINDOW_DAYS` | `7` | Days back for the freshness retriever. Recent chunks in this window get a guaranteed slot in the candidate pool regardless of raw BM25/vector rank. Set to `0` to disable. |
| `rag.freshness_source_types` | — (YAML only) | capture types | Which `source_type` values count for the freshness retriever. Default is the four capture types. Env vars can't represent lists — configure in YAML. |
| — | `RAG_FRESHNESS_PRE_DIVERSITY` | `true` | Apply the freshness multiplier to rerank scores BEFORE the retrieval floor rather than AFTER diversity selection. Stale chunks get dropped by the floor instead of being silently demoted post-hoc. Set to `false` to revert to legacy post-diversity ordering. |
| — | `RAG_RERANK_TITLE_ENRICH` | `true` | Pass chunk title + heading + source/space to the reranker alongside the content body. Title is the single strongest relevance signal and used to be discarded. Set to `false` to send content only (legacy behavior). |
| `rag.entity_cache_ttl_secs` | `RAG_ENTITY_CACHE_TTL_SECS` | `300` | TTL for the entity → space resolution cache. New spaces added to the index become discoverable within this window. |
| `rag.max_rewrites` | `RAG_MAX_REWRITES` | `2` | Maximum alternate queries produced by query rewriting. Each rewrite costs one extra embed call + one extra hybrid search. `0` disables rewriting. |
| `rag.max_chunks_per_doc_in_retriever` | `RAG_MAX_CHUNKS_PER_DOC` | `2` | **Chunk-flood fix.** Max chunks per document that any single retriever may contribute to RRF. Before this knob, BM25 could return 100 chunks of one dominant document, crowding out the real answer. Cap at 2 preserves the top chunk as the RRF anchor plus one more for context. Dedup is per-retriever; different retrievers can still independently vote for the same doc. Set to a large number to effectively disable. |
| — | `RAG_COMPOUND_DECOMPOSE` | `true` | **Compound query decomposition.** Split questions like "what is X and how is X deployed" into distinct sub-intents, rerank each independently against the full candidate pool, and fuse results by taking the max rerank score per chunk across sub-intents. Fixes the class of question where no single chunk answers every intent, so the cross-encoder scores every chunk mediocrely against the compound query. Short questions (<8 words) skip decomposition entirely. Set to `false` to revert to single-query rerank. |

### Grounding floors — what lowering actually costs

The four floor values above (`min_relevance_score`, `display_floor`, `confidence_gate`, `strong_answer_floor`) are the single biggest quality lever in DocBrain. They all gate on the reranker's calibrated `[0, 1]` score, which is the output of stage 3 of the retrieval pipeline. Their defaults are tuned for a real cross-encoder (Cohere Rerank v3.5, Voyage rerank-2, Jina reranker-v2, or equivalent).

**The calibration insight.** A well-tuned cross-encoder's `[0, 1]` scores are **not** a percentage and **not** a uniform distribution. In practice, for Cohere Rerank v3.5 and similar models:

| Score band | What this chunk means for the query |
|---|---|
| `> 0.70` | Directly answers the question. Should be cited. |
| `0.50 – 0.70` | Strongly related, useful supporting evidence. Should be cited. |
| `0.40 – 0.50` | Shares topical overlap. Probably useful context, not a standalone answer. |
| `0.30 – 0.40` | Tangentially related. Shares some keywords. Usually noise. |
| `< 0.30` | Unrelated. Safe to drop. |

The recommended defaults (`0.40 / 0.50 / 0.40 / 0.55`) draw the line at "shares topical overlap" for retrieval and "strongly related" for citation display. That's deliberately asymmetric — the LLM can see weaker evidence than the user sees, so it can reason about it, but we don't surface marginal chunks as if they were endorsed sources.

**The recall-precision knob.** Lowering any floor improves recall (more answers surfaced) and costs precision (more noise in what reaches the user). Raising any floor does the opposite. The four floors target different failure modes:

- **`min_relevance_score` is the strongest lever for hallucination control.** Every chunk above this reaches the LLM. If you set it to `0.0`, the LLM sees the entire candidate pool — including the tangentially-related 30% — and will sometimes write confident-sounding answers grounded in chunks that don't actually support the claim. If you see hallucinations *on questions where the retriever did find the right doc*, this floor is too low.

- **`display_floor` is the strongest lever for citation trust.** Every chunk above this gets shown to the user as a "source". If you see "why is this GitHub PR cited, it has nothing to do with my question?" complaints, this floor is too low. Raising it from `0.30` to `0.50` typically eliminates 60–80% of noisy citations without meaningfully changing answer quality, because the LLM still has access to those chunks internally.

- **`confidence_gate` controls whether sources render at all.** It gates on the **composite** answer confidence, not the top rerank score — that's why it's separate from `strong_answer_floor`. Use it to hide sources on weak answers without killing the answer itself.

- **`strong_answer_floor` is a UX knob, not a retrieval knob.** It only affects whether the answer carries a "low confidence" disclaimer. Lower it if your users find the disclaimer noisy; raise it to make DocBrain more openly uncertain about borderline matches.

**When `rerank.provider = "none"`:** these floors gate on raw BM25/vector scores, which are **not** calibrated to `[0, 1]`. A BM25 score of `0.40` means nothing comparable to a cross-encoder score of `0.40`. Set all four floors to `0.0` in that mode and bound results with `top_k` instead. This is also what makes the plug-and-play rerank providers in [rerank-providers.md](rerank-providers.md) so load-bearing — a real reranker is what makes these floors work at all.

**How to debug a noisy citation.** Run `docbrain trace-query "your question"` and look at the `rerank` log line in stage 3. Each cited chunk has its rerank score printed. If the noisy citation is scoring `0.30–0.45`, it's a floor problem — raise `display_floor` and it goes away. If it's scoring `> 0.50`, the reranker actually thinks it's relevant and the issue is upstream (candidate pool, query decomposition, or title enrichment leaking metadata into the rerank input).

### Observability

Every stage of the pipeline emits a structured log line so you can
trace a single query's path through retrieval without attaching a
debugger:

```
INFO stage="rag.staged.query_understanding" rewrites=2 sub_queries=2 entities=12 mapped_spaces=7
INFO stage="rag.staged.kg_doc_retriever" kg_entities=12 kg_doc_ids=47 hits=18
INFO stage="rag.staged.candidate_generation" retrievers=12 unique_chunks=348 pool_size=200
INFO stage="rag.staged.rrf_fusion" fused=200 rrf_k=60
INFO stage="rag.staged.rerank_sub_query" sub_query="what is payments-svc" top_score=0.82
INFO stage="rag.staged.rerank_sub_query" sub_query="how is payments-svc deployed" top_score=0.79
INFO stage="rag.staged.rerank" input_count=200 output_count=200 top_score=0.82 sub_queries=2 fusion="max_per_chunk"
INFO stage="rag.staged.freshness_pre_diversity" multipliers_fetched=264 reranked_count=200
INFO stage="rag.staged.diversity_select" candidates_in=200 selected=5 top_k=10 max_per_source=3 max_per_document=2 min_relevance_score=0.30
INFO stage="rag.staged.complete" final_count=5 elapsed_ms=7812
```

Stage meanings (in order):

- **query_understanding** — classify intent, extract entities, build rewrites, decompose compound questions into sub-intents, resolve entities to spaces. `sub_queries` is the number of distinct sub-intents the decomposer produced (1 = no decomposition).
- **kg_doc_retriever** — only fires when the knowledge graph has `source_doc_ids` edges for resolved entities. Pulls every chunk of those docs directly, bypassing BM25/vector.
- **candidate_generation** — all retrievers finished. `unique_chunks` is total across the 6–12 retrievers after per-retriever chunk-flood dedup (see `rag.max_chunks_per_doc_in_retriever`).
- **rrf_fusion** — Reciprocal Rank Fusion collapses the retriever outputs into one scored list.
- **rerank_sub_query** — per-sub-query log line emitted in compound-query mode only. Shows the top score that each distinct sub-intent produced against the shared candidate pool.
- **rerank** — cross-encoder scores every chunk against the query. `top_score` in `[0, 1]` is the calibrated highest-ranked hit. Title + heading + space are included in the rerank input when `RAG_RERANK_TITLE_ENRICH=true` (default). When `sub_queries>1`, carries `fusion="max_per_chunk"` indicating each chunk's final score is its best against any sub-intent.
- **freshness_pre_diversity** — fetches stale/fresh multipliers from Postgres and applies them BEFORE the retrieval floor (Phase 1.3, gated on `RAG_FRESHNESS_PRE_DIVERSITY=true`). Stale docs are dropped by the floor instead of being silently demoted post-hoc.
- **diversity_select** — enforces per-source + per-document caps and the retrieval floor. `selected` is the final top-k count.
- **complete** — total wall clock, final_count sent to the LLM.

Set `RAG_TRACE_DETAIL=true` to additionally log every chunk in the
final top-k with its reranker score, space, and document_id. Turn
this on when diagnosing "why didn't chunk X surface?" — the logs will
show whether it was dropped at retrieval, reranking, or diversity
selection.

### Admin trace endpoint — `?trace=true`

Phase 3 adds a structured pipeline trace that admin users can request
per-query instead of grepping logs. POST `/api/v1/ask` with
`{ "question": "...", "stream": false, "trace": true }` and an admin
API key. The response carries an extra `pipeline_trace` field:

```json
{
  "answer": "...",
  "sources": [...],
  "confidence": 0.6,
  "pipeline_trace": {
    "query_id": "7c3a8f9b-...",
    "question": "how is payments-svc deployed in our env?",
    "retrievers_fired": ["literal", "rewrite_0", "entity_space_0", "kg_docs"],
    "pool_size": 200,
    "rerank_provider": "bedrock",
    "sub_queries": ["what is payments-svc", "how is payments-svc deployed in our env"],
    "stage_durations": {
      "query_understanding": 12,
      "kg_doc_retriever": 450,
      "candidate_generation": 1024,
      "rerank": 2870,
      "freshness_pre_diversity": 3,
      "diversity_select": 1,
      "total": 4360
    },
    "chunks": {
      "2217247499_2": {
        "chunk_id": "2217247499_2",
        "document_id": "2217247499",
        "title": "RFC - k8s deployments - A self-service approach of using helm charts",
        "space": "65673",
        "per_retriever_rank": [["kg_docs", 0], ["rewrite_0", 23]],
        "rrf_score": 0.234,
        "rerank_score": 0.72,
        "freshness_multiplier": 0.94,
        "post_freshness_score": 0.677,
        "passed_retrieval_floor": true,
        "passed_diversity": true,
        "final_rank": 0,
        "dropped_at": null
      }
    }
  }
}
```

Non-admin callers with `trace: true` get `pipeline_trace: null` (or
no field, serde skip). No error — the existence of the feature is
hidden from non-admins.

The admin CLI wraps this endpoint:

```
docbrain trace-query "how is payments-svc deployed?"
```

Renders the trace as a table: query info, retrievers fired, per-stage
timings, final top-k chunks with titles and scores. Add `--json` to
dump the raw trace JSON for scripting.

Use this whenever you need to answer "why didn't chunk X surface?"
instead of SSH'ing into the pod and running log-grep pipelines. The
per-stage `dropped_at` field on each chunk names the exact stage that
killed it: `rrf_not_in_pool`, `rerank_below_floor`,
`diversity_source_cap`, `diversity_document_cap`, `diversity_top_k_filled`,
`freshness_penalty`.

### Rolling back

If the staged pipeline ever causes a problem in production, roll back
by setting `RAG_RERANK_PROVIDER=none` in the runtime environment and
restarting the server. No code change, no rebuild, no data migration
— the legacy single-hybrid-search path is byte-identical to before
this feature shipped.

## Document Ingestion

Configure sources in `config/local.yaml` (gitignored). Put only infrastructure secrets in `.env`.

### General

| Setting (`config/local.yaml` key) | Env var equivalent | Default | Description |
|---|---|---|---|
| `ingest.self_ingest` | `DOCBRAIN_SELF_INGEST` | `true` | Auto-ingest DocBrain's own docs |
| `ingest.image_extraction_enabled` | `IMAGE_EXTRACTION_ENABLED` | `true` | Extract and describe images using vision LLM |

Source enablement is structural — a sub-source runs when its block is present
under `sources:` in YAML. There is no separate list or enable flag.

### Local Files

```yaml
# config/local.yaml
sources:
  local:
    path: /data/docs
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `sources.local.path` | `LOCAL_DOCS_PATH` | — | Directory path for local file ingestion |

### Confluence

Set credentials in `config/local.yaml`:

```yaml
confluence:
  base_url: https://yourco.atlassian.net/wiki
  user_email: you@yourco.com
  api_token: ATATT3x...
  space_keys: ENG,DOCS
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `confluence.base_url` | `CONFLUENCE_BASE_URL` | — | Atlassian instance URL (must include `/wiki`) |
| `confluence.user_email` | `CONFLUENCE_USER_EMAIL` | — | Auth email (not required for v1 Data Center) |
| `confluence.api_token` | `CONFLUENCE_API_TOKEN` | — | API token (Cloud) or Personal Access Token (Data Center) |
| `confluence.space_keys` | `CONFLUENCE_SPACE_KEYS` | — | Comma-separated space keys to ingest |
| `confluence.page_limit` | `CONFLUENCE_PAGE_LIMIT` | `0` (unlimited) | Max pages per space. `0` = all pages. |
| `confluence.api_version` | `CONFLUENCE_API_VERSION` | `v2` | `v2` for Cloud, `v1` for Data Center 7.x+ |
| `confluence.tls_verify` | `CONFLUENCE_TLS_VERIFY` | `true` | Set to `false` for self-signed certs |
| `confluence.webhook_secret` | `CONFLUENCE_WEBHOOK_SECRET` | — | HMAC secret for real-time webhook sync (set as env var) |

## Ingestion sources — nested umbrella configuration

All ingestion sources now live under a single top-level `sources:` block. Each
provider has one umbrella entry (`github`, `gitlab`, `slack`, `jira`, `linear`,
…) with its credentials at the top and optional sub-sources nested inside.
A sub-source is **enabled** when its block is present in YAML — there is no
separate `INGEST_SOURCES` env var, and no per-source enable flag.

**Resource lists are always explicit.** Every list-of-targets field (repos,
projects, channels, teams, …) must contain at least one entry. An empty list
is a startup error — DocBrain never silently falls back to "ingest everything
the token can see."

### Selector grammar (GitHub & GitLab)

Repositories are specified with a small selector grammar:

| Syntax | Meaning |
|--------|---------|
| `acme/platform` | Exact repository, use the repo's default branch |
| `acme/platform:develop` | Exact repository, pinned to the `develop` branch |
| `acme/*` | All repositories in the `acme` organisation (default branches) |
| `acme/infra-*` | All `acme` repositories whose name starts with `infra-` |
| `acme/*:main` | **Rejected at startup** — wildcards must use default branches |

> **Wildcards:** Parsing is supported today but **runtime expansion against the
> GitHub/GitLab APIs is a follow-up** and rejected at startup for now with a
> clear error. List repositories explicitly until wildcard resolution lands.

### GitHub (code + pull requests)

```yaml
# config/local.yaml
sources:
  github:
    token: ${GITHUB_TOKEN}                 # repo:read scope
    api_url: https://api.github.com         # override for GitHub Enterprise
    code:                                  # optional — ingest markdown from repos
      repos:
        - acme/platform
        - acme/docs:develop                 # pinned branch
    pull_requests:                         # optional — ingest PR discussions
      repos:
        - acme/platform
        - acme/backend
      lookback_days: 365
      min_comments: 1
      labels: []                            # empty = index all PRs
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `sources.github.token` | `GITHUB_TOKEN` | — | GitHub personal access token with `repo:read` scope |
| `sources.github.api_url` | `GITHUB_API_URL` | `https://api.github.com` | API host override for GitHub Enterprise |
| `sources.github.code.repos` | — | — | **Required when `code` is set.** Non-empty list of `owner/repo[:branch]` selectors |
| `sources.github.pull_requests.repos` | — | — | **Required when `pull_requests` is set.** Non-empty list of `owner/repo` selectors |
| `sources.github.pull_requests.lookback_days` | — | `365` | How far back to fetch merged PRs |
| `sources.github.pull_requests.min_comments` | — | `1` | Minimum total review/issue comments on a PR to be indexed |
| `sources.github.pull_requests.labels` | — | `[]` | Label filter — empty list indexes all PRs |

### GitLab (merge requests)

```yaml
# config/local.yaml
sources:
  gitlab:
    token: ${GITLAB_TOKEN}                 # api scope
    base_url: https://gitlab.com            # override for self-hosted
    tls_verify: true                        # false for self-signed certs
    merge_requests:
      projects:
        - acme/platform
        - acme/infra
      lookback_days: 365
      min_notes: 1
      labels: []
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `sources.gitlab.token` | `GITLAB_TOKEN` | — | GitLab personal or project access token with `api` scope |
| `sources.gitlab.base_url` | `GITLAB_BASE_URL` | `https://gitlab.com` | Instance URL for self-hosted GitLab |
| `sources.gitlab.tls_verify` | `GITLAB_TLS_VERIFY` | `true` | Set to `false` for self-signed certs |
| `sources.gitlab.merge_requests.projects` | — | — | **Required.** Non-empty list of `group/project` paths |
| `sources.gitlab.merge_requests.lookback_days` | — | `365` | How far back to fetch merged MRs |
| `sources.gitlab.merge_requests.min_notes` | — | `1` | Minimum discussion notes on an MR to be indexed |
| `sources.gitlab.merge_requests.labels` | — | `[]` | Label filter — empty list indexes all MRs |

### Slack (threads)

```yaml
# config/local.yaml
sources:
  slack:
    token: ${SLACK_INGEST_TOKEN}           # bot token: channels:history, channels:read, users:read
    threads:
      channels:                             # Slack channel names (not IDs)
        - "#incident-response"
        - "#eng-platform"
      min_replies: 3
      reactions:
        - white_check_mark
        - bookmark
      lookback_days: 90
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `sources.slack.token` | `SLACK_INGEST_TOKEN` | — | Bot token for ingestion (separate from `SLACK_BOT_TOKEN` used by @mentions) |
| `sources.slack.threads.channels` | — | — | **Required.** Non-empty list of channel names (leading `#` optional). The bot must be invited to every channel. |
| `sources.slack.threads.min_replies` | — | `3` | Minimum replies for a thread to be indexed |
| `sources.slack.threads.reactions` | — | `[white_check_mark, bookmark]` | Reactions that override the reply-count threshold |
| `sources.slack.threads.lookback_days` | — | `90` | How far back to scan for threads |

### Jira (issues)

```yaml
# config/local.yaml
sources:
  jira:
    base_url: https://yourcompany.atlassian.net
    user_email: ${JIRA_USER_EMAIL}
    api_token: ${JIRA_API_TOKEN}
    projects:                               # required — no silent "all projects" fallback
      - ENG
      - PLAT
    # jql_filter: "resolution = Fixed"     # optional extra JQL clause
    lookback_days: 365
    issue_types:
      - Bug
      - Story
      - Task
      - Epic
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `sources.jira.base_url` | `JIRA_BASE_URL` | — | Jira instance URL |
| `sources.jira.user_email` | `JIRA_USER_EMAIL` | — | Service-account email for Basic auth |
| `sources.jira.api_token` | `JIRA_API_TOKEN` | — | Atlassian API token |
| `sources.jira.projects` | — | — | **Required.** Non-empty list of project keys (e.g. `ENG`, `PLAT`) |
| `sources.jira.jql_filter` | `JIRA_JQL_FILTER` | — | Additional JQL clause appended to the default query |
| `sources.jira.lookback_days` | `JIRA_LOOKBACK_DAYS` | `365` | How far back to fetch resolved issues |
| `sources.jira.issue_types` | — | `[Bug, Story, Task, Epic]` | Issue types to include |

### Linear (issues)

```yaml
# config/local.yaml
sources:
  linear:
    api_key: ${LINEAR_API_KEY}
    teams:                                  # required — no silent "all teams" fallback
      - ENG
      - OPS
    lookback_days: 365
    states:
      - Done
      - Cancelled
      - Duplicate
```

| Key | Env var | Default | Description |
|-----|---------|---------|-------------|
| `sources.linear.api_key` | `LINEAR_API_KEY` | — | Linear personal API key |
| `sources.linear.teams` | — | — | **Required.** Non-empty list of team keys |
| `sources.linear.lookback_days` | `LINEAR_LOOKBACK_DAYS` | `365` | How far back to fetch completed/cancelled issues |
| `sources.linear.states` | — | `[Done, Cancelled, Duplicate]` | Issue states to include |


## Rate Limiting

DocBrain applies per-IP rate limiting to unauthenticated routes and per-API-key rate limiting to authenticated routes. Rate limiting is enabled by default.

| Variable | Default | Description |
|----------|---------|-------------|
| `RATE_LIMIT_ENABLED` | `true` | Set to `false` to disable all rate limiting (not recommended for production) |
| `RATE_LIMIT_RPM` | `60` | Requests per minute per IP on unauthenticated routes |
| `RATE_LIMIT_AUTH_RPM` | `120` | Requests per minute per API key on authenticated routes |
| `RATE_LIMIT_WEBHOOK_RPM` | `30` | Requests per minute per IP on webhook endpoints (`/github/events`, `/gitlab/events`) |

When a rate limit is exceeded, DocBrain returns `429 Too Many Requests` with a `Retry-After` header.

## GitLab MR Capture Webhook

The GitLab capture feature lets engineers trigger immediate ingestion by commenting `@docbrain capture` on any merge request.

| Variable | Default | Description |
|----------|---------|-------------|
| `GITLAB_CAPTURE_WEBHOOK_SECRET` | — | HMAC secret shared with GitLab for webhook signature verification |
| `GITLAB_CAPTURE_TOKEN` | — | GitLab personal access token with `api` scope (fetches MR notes and posts reply comments) |
| `GITLAB_CAPTURE_BASE_URL` | `https://gitlab.com` | GitLab instance base URL (override for self-hosted) |
| `GITLAB_CAPTURE_ALLOWED_USERS` | — | Comma-separated GitLab usernames allowed to trigger capture. Empty = all users. |
| `GITLAB_CAPTURE_ALLOWED_PROJECTS` | — | Comma-separated project paths allowed to trigger capture. Empty = all projects. e.g. `myorg/myrepo` |

See [Ingestion Guide](ingestion.md#gitlab-mr-capture) for full setup instructions.

## GitHub Capture Security

These optional variables restrict which repos and users can trigger real-time GitHub PR/issue capture via `@docbrain capture` comments.

| Variable | Default | Description |
|----------|---------|-------------|
| `GITHUB_CAPTURE_ALLOWED_REPOS` | — | Comma-separated `owner/repo` pairs allowed to trigger capture. Empty = all repos. e.g. `myorg/backend,myorg/frontend` |
| `GITHUB_CAPTURE_ALLOWED_USERS` | — | Comma-separated GitHub usernames allowed to trigger capture. Empty = all users. e.g. `alice,bob` |

A 500KB content size guard applies to all capture requests. Oversized threads are rejected with a reply comment.

## Confluence Webhooks (Real-Time Sync)

| Variable | Default | Description |
|----------|---------|-------------|
| `CONFLUENCE_WEBHOOK_SECRET` | — | HMAC secret shared with Confluence. When set, DocBrain mounts `POST /confluence/events` and auto-ingests page changes in real time. Set as an environment variable (not in `config/local.yaml`). |

When configured, DocBrain receives `page_created`, `page_updated`, `page_restored`, `page_removed`, and `page_trashed` events from Confluence and syncs changes automatically — no scheduled re-ingest needed.

Requires `confluence.base_url` and `confluence.api_token` to also be set in `config/local.yaml` (DocBrain needs API access to fetch the page content when a webhook fires).

See the [Ingestion Guide](ingestion.md#real-time-sync-confluence-webhooks) for setup instructions.

## Image Extraction

| Variable | Default | Description |
|----------|---------|-------------|
| `IMAGE_EXTRACTION_ENABLED` | `true` | Extract and describe images from Confluence pages using vision LLM. Set to `false` to disable. |
| `INGEST_LLM_MODEL_ID` | — | Model used for image extraction during ingest. Falls back to `LLM_MODEL_ID` if not set. Set this to a cheaper model (Haiku, `gpt-4o-mini`) to avoid throttling and reduce cost. |
| `IMAGE_MAX_PER_PAGE` | `20` | Maximum images to process per Confluence page |
| `IMAGE_MIN_SIZE_BYTES` | `5120` | Skip images smaller than this in bytes (default: 5 KB) — filters out icons and decorative images |
| `IMAGE_MAX_SIZE_BYTES` | `10485760` | Skip images larger than this in bytes (default: 10 MB) |
| `IMAGE_DOWNLOAD_TIMEOUT` | `30` | HTTP download timeout in seconds per image |
| `IMAGE_LLM_TIMEOUT` | `120` | LLM vision call timeout in seconds (needs more time than download) |

Image extraction requires a vision-capable LLM. Supported providers: **Bedrock**, **Anthropic**, **OpenAI**, and **Ollama** (with vision models like `llava`, `llama3.2-vision`, `moondream`). Text-only models (e.g. `llama3.1`) are auto-detected and images are skipped gracefully — no failures, no errors.

## Web UI / CORS

| Variable | Default | Description |
|----------|---------|-------------|
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3001` | Comma-separated origins allowed to call the API. Only needed if the web UI is served from a non-default origin (e.g. `http://10.0.0.5:3001`, `https://docbrain.internal`) |

> **Note:** The default works out of the box for Docker Compose. You only need this if you access the web UI via a different hostname or port — for example, `http://127.0.0.1:3001` is a different origin than `http://localhost:3001`.

## Auth / Sessions

| Variable | Default | Description |
|----------|---------|-------------|
| `LOGIN_SESSION_TTL_HOURS` | `720` | Session lifetime after email/password login (default: 720 hours = 30 days). Set to `0` for no expiry. |
| `MAX_QUERY_LENGTH` | `4000` | Maximum characters allowed for question and description inputs |

## Slack Integration (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `SLACK_BOT_TOKEN` | — | Slack bot OAuth token (`xoxb-...`) |
| `SLACK_SIGNING_SECRET` | — | Slack app signing secret |
| `SLACK_GAP_NOTIFICATION_CHANNEL` | — | Channel to post critical gap alerts after each analysis run (e.g. `#docs-alerts`). Only fires when new critical-severity gaps are found. Requires `SLACK_BOT_TOKEN`. |

## Notifications (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `NOTIFICATION_INTERVAL_HOURS` | `24` | How often to check for stale docs and send owner DMs |
| `NOTIFICATION_SPACE_FILTER` | — | Comma-separated spaces to limit notifications (e.g. `PLATFORM,SRE`). Empty = all spaces. |

## Documentation Autopilot (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `AUTOPILOT_ENABLED` | `false` | Enable the Documentation Autopilot (gap detection + draft generation) |
| `AUTOPILOT_GAP_ANALYSIS_INTERVAL_HOURS` | `6` | How often the background scheduler runs gap analysis |
| `AUTOPILOT_LOOKBACK_DAYS` | `30` | Days of query history to analyse for gaps |
| `AUTOPILOT_CLUSTER_THRESHOLD` | `0.82` | Cosine similarity threshold for grouping queries into a gap cluster (0.65 = loose, 0.85 = strict) |
| `AUTOPILOT_MIN_CLUSTER_SIZE` | `3` | Minimum episodes in a cluster to be considered a real gap |
| `AUTOPILOT_MIN_UNIQUE_USERS` | `2` | Minimum distinct users that must hit the same gap topic |
| `AUTOPILOT_MIN_NEGATIVE_RATIO` | `0.15` | Minimum fraction of queries on a topic that must have negative feedback |
| `AUTOPILOT_MAX_CLUSTERS` | `50` | Maximum gap clusters to persist per analysis run |
| `AUTOPILOT_MAX_EPISODES` | `500` | Maximum negative episodes to load per analysis run |
| `AUTOPILOT_AUTO_DRAFT` | `false` | Automatically generate drafts for qualifying gaps (no human trigger). Set to `true` to enable. |
| `AUTOPILOT_AUTO_DRAFT_SEVERITY` | `critical` | Minimum gap severity for auto-drafting: `critical`, `high`, `medium`, or `low` |
| `AUTOPILOT_CRITICAL_USERS` | `5` | Unique users needed for breadth score to reach 1.0. Lower for small teams. |
| `AUTOPILOT_CRITICAL_SIGNALS` | `15` | Negative signals needed for volume score to reach 1.0. Lower for low-traffic deployments. |
| `AUTOPILOT_CRITICAL_THRESHOLD` | `0.75` | Composite score cutoff for "critical" severity. |
| `AUTOPILOT_HIGH_THRESHOLD` | `0.55` | Composite score cutoff for "high" severity. |
| `AUTOPILOT_MEDIUM_THRESHOLD` | `0.35` | Composite score cutoff for "medium" severity. |

When enabled, Autopilot runs on the configured schedule, exposes management endpoints at `/api/v1/autopilot/*`, and posts critical gap alerts to `SLACK_GAP_NOTIFICATION_CHANNEL` if configured. See the [API Reference](api-reference.md) for endpoint details.

> **Small teams / dev environments:** Set `AUTOPILOT_CRITICAL_USERS=1`, `AUTOPILOT_CRITICAL_SIGNALS=3`, `AUTOPILOT_CRITICAL_THRESHOLD=0.3` to see critical gaps with minimal signal. See [autopilot.md](autopilot.md) for a full tuning guide.

## Draft Publishing

Controls where AI-generated drafts are published. Supports Confluence (default), GitHub (PR-based), and GitLab (MR-based). Use per-space routing via the Publish Targets API to override the default target for specific spaces.

| Variable | Default | Description |
|----------|---------|-------------|
| `DRAFT_PUBLISH_TARGET` | `none` | Default publish target: `confluence`, `github`, `gitlab`, or `none` |
| `DRAFT_PUBLISH_AUTO_INGEST` | `true` | Re-ingest published docs so DocBrain learns from its own output |

### GitHub Publishing

Publish drafts as Pull Requests containing markdown files with YAML frontmatter. Requires a GitHub token with `repo` scope.

| Variable | Default | Description |
|----------|---------|-------------|
| `GITHUB_PUBLISH_TOKEN` | — | GitHub personal access token with `repo` scope (secret) |
| `GITHUB_PUBLISH_REPO` | — | Target repository in `owner/repo` format (e.g. `acme/docs`) |
| `GITHUB_PUBLISH_BRANCH` | `main` | Base branch for PRs |
| `GITHUB_PUBLISH_DOCS_PATH` | `docs` | Directory in repo where doc files are placed |
| `GITHUB_PUBLISH_PR_LABELS` | `docbrain,auto-generated` | Comma-separated labels applied to PRs |
| `GITHUB_PUBLISH_CREATE_PR` | `true` | `true` = create a PR for review; `false` = commit directly to branch |
| `GITHUB_PUBLISH_API_URL` | `https://api.github.com` | Override for GitHub Enterprise Server |

### GitLab Publishing

Publish drafts as Merge Requests containing markdown files. Requires a GitLab token with `api` scope.

| Variable | Default | Description |
|----------|---------|-------------|
| `GITLAB_PUBLISH_TOKEN` | — | GitLab personal access token with `api` scope (secret) |
| `GITLAB_PUBLISH_PROJECT_ID` | — | Numeric project ID (find in Settings → General) |
| `GITLAB_PUBLISH_BASE_URL` | `https://gitlab.com` | Override for self-hosted GitLab instances |
| `GITLAB_PUBLISH_BRANCH` | `main` | Base branch for MRs |
| `GITLAB_PUBLISH_DOCS_PATH` | `docs` | Directory in project where doc files are placed |
| `GITLAB_PUBLISH_MR_LABELS` | `docbrain,auto-generated` | Comma-separated labels applied to MRs |
| `GITLAB_PUBLISH_CREATE_MR` | `true` | `true` = create an MR for review; `false` = commit directly to branch |

### Per-Space Routing

Use the Publish Targets API (`/api/v1/publish-targets`) to route specific spaces to different targets. For example, keep Confluence as the default but publish the `PLATFORM` space to GitHub:

```bash
# Create a GitHub target for the PLATFORM space
curl -X POST /api/v1/publish-targets \
  -H "Authorization: Bearer db_sk_..." \
  -d '{"space": "PLATFORM", "target_type": "github", "config": {"token_env": "GITHUB_PUBLISH_TOKEN", "repo": "acme/platform-docs"}, "priority": 10}'
```

When publishing, DocBrain resolves the target in priority order: space-specific DB target → default config target → Confluence fallback. Config stored in the `publish_targets` table uses `token_env` (env var name) instead of raw secrets for security.

## Freshness Scoring

| Variable | Default | Description |
|----------|---------|-------------|
| `FRESHNESS_SCHEDULER_INTERVAL_HOURS` | `24` | How often freshness scores are recalculated for all documents |
| `CONTRADICTION_CHECKS_PER_PASS` | `10` | Max documents checked for contradictions per freshness run (LLM cost) |
| `CONTRADICTION_INCLUDE_RECENT_EVENT_DOCS` | `true` | Include recent Slack/PR/Jira docs in the contradiction pass alongside stalest docs |
| `CONTRADICTION_EVENT_DOC_MAX_AGE_DAYS` | `90` | Only event-based docs edited within this many days are eligible for contradiction checks |

### Event-Based Source Types

Source types whose documents are permanent historical records — incident threads, merged PRs, support tickets — never go stale and shouldn't be evaluated for content currency or contradictions. The scorer pins their `time_decay = 100` and skips LLM/link/contradiction passes.

This was a hardcoded list until v1.4; it's now configurable so operators can register custom permanent-record source types (e.g. a homegrown incident system) without rebuilding the image.

| YAML key (under `freshness`) | Default | Description |
|------------------------------|---------|-------------|
| `event_based_spaces` | `[slack_thread, github_pr, github, gitlab_mr, jira, linear, pagerduty, opsgenie, zendesk, intercom, fireflies]` | List of `documents.space` values treated as permanent historical records. Capture sources (`slack_capture`, `github_capture`, `gitlab_capture`) are intentionally NOT in the default — design discussions DO go stale. |

Override in `default.yaml` (or via the helm value `freshness.eventBasedSpaces`) to add custom source types.

### Lifecycle Exclusion (archived/historical documents)

Documents that are intentionally frozen — archived project pages, historical decision records, reference material — should not be evaluated for freshness. DocBrain auto-detects these from source-system metadata at ingest and skips them in scoring.

**How it works.** During Confluence ingestion DocBrain reads each page's labels and (for Confluence Cloud) page status. If any match the rules below, the doc's `lifecycle_status` becomes `archived` and the freshness scorer skips it entirely. Manual overrides via `PATCH /api/v1/documents/:id/lifecycle` are sticky — they survive future syncs.

| YAML key (under `freshness.exclusion_rules`) | Default | Description |
|----------------------------------------------|---------|-------------|
| `archived_labels` | `[archived, historical, obsolete, deprecated, frozen, reference]` | Source labels (case-insensitive). Confluence page labels match here. |
| `archived_page_statuses` | `[archived, trashed]` | Confluence Cloud page-status values that mark a doc as archived. |
| `archived_title_patterns` | `['^Archived ', '^\[ARCHIVED\]', '\(archived\)$']` | Regex patterns matched against doc title — safety net for un-labeled legacy docs. |

These rules are list-valued and configured in YAML only (env vars can't represent lists). To exclude a doc on demand without changing source-system labels, use the lifecycle API.

### Semantic Quality Scoring

LLM-based quality assessment that evaluates documents on four dimensions: accuracy, completeness, clarity, and actionability (each scored 0-25, total 0-100). Runs as a background sweep on documents that have already been structurally scored.

| Variable | Default | Description |
|----------|---------|-------------|
| `SEMANTIC_QUALITY_ENABLED` | `true` | Enable LLM-based semantic quality scoring |
| `SEMANTIC_QUALITY_INTERVAL_HOURS` | `24` | How often the semantic scoring sweep runs |
| `SEMANTIC_QUALITY_BUDGET` | `50` | Maximum documents scored per sweep (controls LLM cost) |
| `SEMANTIC_QUALITY_STRUCTURAL_THRESHOLD` | `40.0` | Minimum structural score required before a document is eligible for semantic scoring |

The composite quality score blends structural and semantic scores at 50/50 weighting. Documents below the structural threshold are skipped to avoid wasting LLM calls on obviously poor content.

### Capture Lifecycle

Captured content (GitHub PRs/issues, GitLab MRs, Slack threads) decays with age — unlike incident records (Jira, PagerDuty, Zendesk) which are permanent historical events. A 5-year-old PR discussing a replaced architecture should score low in freshness; a 2-week-old incident thread is always valid.

**Cross-document references:** During capture, DocBrain automatically extracts URLs from the description and comments — GitHub PRs, GitLab MRs, Jira tickets, Confluence pages, and other linked resources. These are stored as a reference graph in PostgreSQL and used to enrich RAG context at query time by fetching chunks from referenced documents. GitLab shorthand references (`!123` for MRs, `#123` for issues) are resolved to full URLs within the same project.

**Space assignment:** Captures are stored under a meaningful space name derived from the source:
- GitHub captures → `owner/repo` (e.g., `myorg/backend`)
- GitLab captures → `group/project` (e.g., `platform/api`)
- Slack captures → channel name (e.g., `platform-incidents`)

This makes `allowed_spaces` ACL filtering work correctly — a key scoped to `["myorg/backend"]` will include GitHub captures from that repo.

**Age baseline:** Freshness is calculated from the **original content creation date** (when the PR was opened, when the Slack thread started) — not the time DocBrain captured it. Re-capturing the same thread updates its content but preserves the original creation date as the staleness baseline.

## Memory Consolidation

| Variable | Default | Description |
|----------|---------|-------------|
| `CONSOLIDATION_INTERVAL_HOURS` | `6` | How often the memory consolidation job runs (merges episodic patterns into semantic/procedural memory) |

## RAG Pipeline

| Variable | Default | Description |
|----------|---------|-------------|
| `RAG_TOP_K` | `10` | Chunks retrieved per query. Higher = more context passed to the LLM, at the cost of more tokens per call. Raise to `15`–`20` if answers are missing obvious information; lower to `5` to reduce cost on simple corpora. |
| `RAG_BM25_BOOST` | `1.0` | Weight of keyword (BM25) search relative to vector search in hybrid retrieval. Raise to `2.0`–`3.0` for corpora heavy with exact-match queries — error codes, CLI commands, ticket IDs, specific tool names. Leave at `1.0` for general prose documentation. |
| `SEARCH_MIN_SCORE` | `0.0` | Drop retrieved chunks below this relevance score before sending context to the LLM. `0.0` keeps everything. Set to `0.3`–`0.4` if you notice irrelevant chunks contaminating answers; leave at `0.0` for small corpora where recall matters more than precision. |
| `RAG_CACHE_TTL_HOURS` | `24` | How long to cache semantically identical answers |
| `RAG_CACHE_THRESHOLD` | `0.95` | Cosine similarity threshold for a query to count as a cache hit |

## Chunking

Controls how documents are split before embedding. See [Ingestion Guide](ingestion.md) for re-ingest instructions.

| Variable | Default | Description |
|----------|---------|-------------|
| `CHUNK_SIZE` | `1500` | Target chunk size in characters. Dense API refs: `800`–`1200`. General docs: `1500`. Long-form prose: `2000`–`2500`. |
| `CHUNK_OVERLAP` | `200` | Overlap between adjacent paragraph-split chunks in characters. |

## OpenSearch Index Names

| Variable | Default | Description |
|----------|---------|-------------|
| `OPENSEARCH_INDEX` | `docbrain-chunks` | Index name for document chunks (vectors + BM25) |
| `OPENSEARCH_EPISODE_INDEX` | `docbrain-episodes` | Index name for episode vectors (used in episodic memory recall) |

Only change these if you run multiple DocBrain instances sharing the same OpenSearch cluster, to avoid index collisions.

## Data Retention

| Variable | Default | Description |
|----------|---------|-------------|
| `EPISODE_RETENTION_DAYS` | `90` | Episode (query history) rows older than this are pruned daily. Set to `0` to disable pruning. |
| `AUDIT_RETENTION_DAYS` | `365` | Audit log rows older than this are pruned daily. Set to `0` to disable pruning. |

## Self-Ingest (Optional)

| Variable | Default | Description |
|----------|---------|-------------|
| `DOCBRAIN_SELF_INGEST` | `true` | Auto-ingest DocBrain's own docs so it can answer configuration questions about itself |
| `DOCBRAIN_DOCS_PATH` | `./docs` | Path to DocBrain's own documentation directory |

## SSO / OIDC (Enterprise)

| Variable | Default | Description |
|----------|---------|-------------|
| `OIDC_ISSUER_URL` | — | OIDC provider URL (e.g. `https://accounts.google.com`) |
| `OIDC_CLIENT_ID` | — | OAuth client ID |
| `OIDC_CLIENT_SECRET` | — | OAuth client secret |
| `OIDC_REDIRECT_URI` | — | Callback URI (e.g. `https://docbrain.example.com/api/v1/auth/oidc/callback`) |
| `OIDC_WEB_UI_URL` | `http://localhost:3001` | Where to redirect after successful login |
| `OIDC_ACCEPT_INVALID_CERTS` | `false` | Set to `true` to skip TLS verification — use for corporate/self-signed CAs |

### GitLab OIDC

| Variable | Default | Description |
|----------|---------|-------------|
| `GITLAB_OIDC_ISSUER_URL` | — | GitLab instance URL (e.g. `https://gitlab.com` or `https://gitlab.corp.example.com`) |
| `GITLAB_CLIENT_ID` | — | GitLab OAuth application client ID |
| `GITLAB_CLIENT_SECRET` | — | GitLab OAuth application client secret |
| `GITLAB_REDIRECT_URI` | — | Callback URL (e.g. `https://docbrain.example.com/api/v1/auth/gitlab/callback`) |

> **Corporate GitLab:** If your self-hosted GitLab uses an internal CA, set `OIDC_ACCEPT_INVALID_CERTS=true`.

---

## RBAC Role Assignment

Role is computed at login time and stored on the user record. The hierarchy is: `viewer (1) < editor (2) < analyst (3) < admin (4)`. Higher-priority rules win.

| Variable | Helm key | Description |
|----------|----------|-------------|
| `OIDC_DEFAULT_ROLE` | `rbac.defaultRole` | Role assigned to new SSO users who match no group rule. Default: `viewer`. |
| `OIDC_ADMIN_EMAILS` | `rbac.adminEmails` | Comma-separated emails that always receive `admin`. |
| `OIDC_ADMIN_DOMAIN` | `rbac.adminDomain` | Email domain whose users receive `admin` (e.g. `acme.com`). |
| `OIDC_ADMIN_GROUPS` | `rbac.adminGroups` | Comma-separated IdP group names → `admin` role. |
| `OIDC_EDITOR_GROUPS` | `rbac.editorGroups` | Comma-separated IdP group names → `editor` role. |
| `OIDC_ALLOWED_GROUPS` | `rbac.allowedGroups` | Access gate: only these groups may log in (all others get 403). |
| `OIDC_ALLOWED_DOMAINS` | `rbac.allowedDomains` | Access gate: only these email domains may log in. |

### What every engineer can see

All authenticated users (including `viewer`) have full access to the intelligence dashboards:

| Page | What it shows |
|------|---------------|
| **Velocity** | Documentation ROI — queries deflected, hours saved, cost saved, per-team breakdown |
| **Predictive** | Predicted documentation gaps from code changes, cascade staleness, seasonal patterns, onboarding risks |
| **Maintenance** | AI-generated fix proposals with apply/reject workflow |
| **Stream** | Live knowledge event feed — incident warnings, freshness decay alerts, trending gaps |

These dashboards are visible to every engineer. The insight loop only works if the people who can act on it — the engineers — can actually see it.

**Example — typical multi-team setup:**

```yaml
rbac:
  defaultRole: "viewer"
  adminGroups: "platform-team"
  editorGroups: "docs-writers"
```

```bash
# Equivalent env vars
OIDC_DEFAULT_ROLE=viewer
OIDC_ADMIN_GROUPS=platform-team
OIDC_EDITOR_GROUPS=docs-writers
```

> **Note:** Role is evaluated at login time. Group changes in your IdP take effect on next login.

---

## ACL

Mirrors source-system permissions (Confluence space restrictions, Slack private channels, GitHub repo visibility, Jira issue security levels) at query time. A user only sees retrieval results for documents they can read in the source.

For the conceptual guide, modes, denial UX, audit log, and threat model, see **[Access Control (ACL)](access-control.md)**. The reference below is the env-var / YAML surface only.

### Top-level

| Variable | Default | Description |
|----------|---------|-------------|
| `ACL_MODE` | `off` | `off` (no filtering), `warn` (log denials, return all), `enforce` (filter + redact) |
| `ACL_RECALL_OVERFETCH` | `2.0` | Recall multiplier — pull this much extra from the index so post-filter results still hit `top_k` |
| `ACL_UNKNOWN_POLICY` | `deny` | What to do with chunks that have no ACL data: `deny` (fail-closed) or `allow` (legacy / migration mode) |

### Per-source policy (`acl.sources.*`)

Each connector slot accepts `mirror` (default — use real source ACLs), `public` (everyone in the workspace can see all docs from this source), or `admin_only`.

```yaml
acl:
  sources:
    confluence: mirror
    slack: mirror
    github: mirror
    jira: mirror
    gitlab: public        # if your GitLab MRs are intentionally workspace-wide
    ms_teams: admin_only  # restrict until ACL provider lands
    linear: mirror
```

Per-namespace overrides (per Confluence space, per Slack channel, etc.) live under `acl.denial.source_overrides.<source>.{space,channel,repo,project}_overrides`.

### Denial UX (`acl.denial.*`)

| Variable | Default | Description |
|----------|---------|-------------|
| `ACL_DENIAL_MODE` | `disclosed_no_count` | `silent` (no hint), `disclosed_no_count` (acknowledge, hide count), `disclosed` (full count + breakdown) |
| `ACL_DENIAL_REFERRAL` | _unset_ | Optional URL shown in denial messages (e.g. your access-request portal) |
| `ACL_DENIAL_PARTIAL_DENIAL` | `true` | Surface `access` metadata even when *some* results were returned |
| `ACL_AUDIT_ENABLED` | `false` | Write denial events to `acl_audit_log` (required for HIPAA / FedRAMP / SOC2 trails) |
| `ACL_AUDIT_RAW_QUERY` | `false` | Store the raw user query (default: SHA256 hash only — queries can carry MNPI / PII) |

Per-role overrides (admin sees full disclosure, employee sees no count) and per-source overrides are YAML-only:

```yaml
acl:
  denial:
    mode: disclosed_no_count
    role_overrides:
      admin: disclosed
    source_overrides:
      confluence:
        mode: disclosed
      slack:
        mode: silent
```

> Strictest-wins: if any one denied source resolves to `silent`, the whole response goes silent. This prevents side-channel leaks where a user learns *which* source restricted them.

### Diagnostics

```bash
# What does ACL think this user can see?
GET /api/v1/me/acl

# Coverage report — how many indexed chunks have ACL principals attached?
SELECT source_type, COUNT(*) FROM document_acl GROUP BY source_type;
```

---

## Documentation Analytics

| Variable | Default | Description |
|----------|---------|-------------|
| `VELOCITY_MINUTES_SAVED_PER_QUERY` | `15` | Estimated minutes saved per deflected query |
| `VELOCITY_HOURLY_RATE` | `75` | Effective hourly rate (USD) for ROI calculation |

---

## Knowledge Stream

| Variable | Default | Description |
|----------|---------|-------------|
| `STREAM_ENABLED` | `false` | Enable background knowledge stream emission |
| `STREAM_INTERVAL_MINUTES` | `30` | How often the stream background task runs |
| `STREAM_INCIDENT_WARNING_MIN_USERS` | `2` | Minimum unique users hitting an unanswered question to emit an incident warning |
| `STREAM_DECAY_THRESHOLD` | `0.5` | Freshness score below which a decay alert is emitted |

## Event Bus

The event bus is internal pub/sub infrastructure — always enabled, no opt-in required. Every significant action (document ingest, gap detection, draft generation, etc.) emits a typed event that subscribers can react to.

| Variable | Default | Description |
|----------|---------|-------------|
| `EVENT_BUS_CAPACITY` | `4096` | Broadcast channel buffer size. Increase if subscribers lag under high event volume. Max: 65536. |
| `EVENT_LOG_RETENTION_DAYS` | `90` | Days to retain events in the `event_log` table before purging. |

**Admin API endpoints:**

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/api/v1/events` | Query the persistent event log. Supports `?type=gap.detected&since=2026-03-01&limit=100&offset=0`. |
| `GET` | `/api/v1/events/stream` | SSE stream of real-time events. Max 10 concurrent connections. |

Both endpoints require **admin** role.

## Knowledge Fragments

Knowledge fragments are first-class units of knowledge — smaller than documents, richer than chunks. They capture decisions, facts, caveats, procedures, and context from PRs, commits, IDE annotations, conversations, CI/CD pipelines, and manual entry.

Fragments are routed by confidence score: high-confidence fragments are auto-indexed into search, medium-confidence go to a review queue, and low-confidence are auto-discarded.

| Variable | Default | Description |
|----------|---------|-------------|
| `FRAGMENT_AUTO_INDEX_THRESHOLD` | `0.7` | Minimum confidence score to auto-index a fragment into OpenSearch. |
| `FRAGMENT_REVIEW_THRESHOLD` | `0.4` | Minimum confidence for the review queue. Fragments below this are auto-discarded. |
| `FRAGMENT_MAX_CONTENT_LENGTH` | `10000` | Maximum fragment content length in characters. |

## Fragment Clustering & Auto-Composition

Semantic clustering groups related fragments by topic using embedding similarity (DBSCAN-style greedy algorithm). When a cluster meets composability criteria (5+ fragments, diverse sources, 500+ words), it can be auto-composed into a documentation draft via the API.

| Variable | Default | Description |
|----------|---------|-------------|
| `FRAGMENT_CLUSTERING_ENABLED` | `true` | Enable or disable the fragment clustering endpoint. |
| `FRAGMENT_CLUSTER_THRESHOLD` | `0.80` | Cosine similarity threshold for grouping fragments (0.60 = loose, 0.90 = strict). |
| `FRAGMENT_MIN_CLUSTER_SIZE` | `3` | Minimum fragments required to form a cluster. |
| `FRAGMENT_MIN_SOURCE_DIVERSITY` | `2` | Minimum distinct source types for a cluster to be composable. |
| `FRAGMENT_MAX_PER_CLUSTERING_RUN` | `2000` | Maximum fragments loaded per clustering run (memory/cost control). |

## CI/CD Pipeline Capture

Automated knowledge extraction from merged PRs and deployments. When enabled, DocBrain provides API endpoints that CI/CD pipelines can call to extract knowledge fragments from pull requests and deployment events. Uses the fast/cheap LLM model to keep costs low at high volume.

| Variable | Default | Description |
|----------|---------|-------------|
| `CI_ANALYZE_ENABLED` | `true` | Enable or disable the CI/CD capture endpoints (`/api/v1/ci/analyze` and `/api/v1/ci/deploy-capture`). |

See the [API Reference](api-reference.md#cicd-pipeline-capture) for endpoint details and the GitHub Action setup guide.

## Conversation Auto-Distillation

Automatically extracts structured knowledge fragments from captured conversations — Slack threads (via message shortcut, `@DocBrain capture`, or `/docbrain capture`) and GitHub PR discussions (via `@docbrain capture`). After a successful capture, DocBrain runs LLM-powered distillation in the background to identify decisions, facts, caveats, procedures, and context embedded in the conversation.

Distillation is fire-and-forget: it never affects capture response time. Failures are logged and metriced but don't block the capture path.

| Variable | Default | Description |
|----------|---------|-------------|
| `DISTILLATION_ENABLED` | `true` | Enable or disable conversation auto-distillation. |
| `DISTILLATION_MAX_CONCURRENT` | `3` | Maximum concurrent LLM distillation calls (bounded by semaphore). |
| `DISTILLATION_MAX_CONTENT_CHARS` | `8000` | Maximum conversation characters sent to the LLM. Longer conversations are truncated (tail-biased — keeps the most recent messages). |
| `DISTILLATION_MAX_FRAGMENTS` | `5` | Maximum knowledge fragments extracted per conversation. |

## Governance SLA Checker

The SLA checker runs as a periodic background task that detects breaches across four entity types: gap acknowledgment, gap resolution, draft review, and document freshness. SLA thresholds are stored in the database (per-space overridable via the API) — these settings control the checker's operational behavior.

| Variable | Default | Description |
|----------|---------|-------------|
| `SLA_CHECKER_INTERVAL_HOURS` | `1` | How often the SLA breach checker runs (hours). |
| `SLA_CHECKER_QUERY_TIMEOUT_SECS` | `30` | Per-entity-type query timeout in seconds. |
| `SLA_CHECKER_MAX_CANDIDATES` | `5000` | Maximum candidate entities scanned per type per run. |
| `SLA_CHECKER_MAX_EVENTS_PER_RUN` | `50` | Maximum `SlaBreached` events emitted per run (prevents webhook flooding). |

See the [API Reference — Governance SLAs](api-reference.md#governance-slas) for endpoint documentation.

## External Connectors (HTTP Connector Protocol)

External connectors are stateless HTTP servers that implement a simple REST contract (`GET /health`, `POST /documents/list`, `POST /documents/fetch`). DocBrain calls them on a configurable cron schedule to ingest documents from external systems. Connectors are registered and managed via the admin API.

The connector scheduler runs as a background task, polling every 60 seconds for connectors whose cron schedule is due. A circuit breaker automatically disables connectors after repeated failures.

| Variable | Default | Description |
|----------|---------|-------------|
| `CONNECTOR_ENABLED` | `true` | Enable/disable the connector scheduler |
| `CONNECTOR_MAX_CONCURRENT_SYNCS` | `3` | Max connectors syncing simultaneously (1-20) |
| `CONNECTOR_MAX_PAGES_PER_SYNC` | `200` | Max list pages fetched per sync |
| `CONNECTOR_MAX_DOCUMENTS_PER_SYNC` | `5000` | Max documents ingested per sync |
| `CONNECTOR_FETCH_BATCH_SIZE` | `50` | Documents fetched per batch (1-200) |
| `CONNECTOR_REQUEST_TIMEOUT_SECS` | `30` | HTTP timeout for individual connector requests (5-300 seconds) |
| `CONNECTOR_SYNC_TIMEOUT_SECS` | `3600` | Overall sync timeout per connector (60-7200 seconds) |
| `CONNECTOR_MAX_RESPONSE_BYTES` | `10485760` | Max response body size from connector (10 MB) |
| `CONNECTOR_CIRCUIT_BREAKER_THRESHOLD` | `5` | Consecutive failures before auto-disabling a connector |
| `CONNECTOR_ALLOW_INTERNAL` | `false` | Allow connector URLs on private/internal IP addresses. **Not recommended for production.** |

See the [API Reference — Connectors](api-reference.md#external-connectors) for endpoint documentation and the connector protocol spec.

## Webhooks (Outbound)

Outbound webhook subscriptions let you push DocBrain events to external systems — Slack bots, CI/CD pipelines, PagerDuty, custom dashboards, etc. DocBrain signs every delivery with HMAC-SHA256, retries with exponential backoff, and automatically disables subscriptions that fail repeatedly (circuit breaker).

| Variable | Default | Description |
|----------|---------|-------------|
| `WEBHOOK_DELIVERY_TIMEOUT_SECONDS` | `10` | HTTP timeout per webhook delivery attempt (1-60 seconds) |
| `WEBHOOK_MAX_RETRIES` | `4` | Maximum delivery attempts before giving up (1-10) |
| `WEBHOOK_CIRCUIT_BREAKER_THRESHOLD` | `10` | Consecutive failures before auto-disabling a subscription (3-100) |
| `ALLOW_INTERNAL_WEBHOOKS` | `false` | Allow delivery to private/internal IP addresses (10.x, 172.16.x, 192.168.x). **Not recommended for production.** |

See the [API Reference — Webhooks](api-reference.md#webhooks) for endpoint documentation and event types.

## Style Rules Engine

The style rules engine provides configurable linting for documentation consistency. Rules are always enabled — no opt-in required. Rules are managed via the API (CRUD + YAML import/export) and stored in PostgreSQL.

Rules are scoped either globally (`space = null`) or per-space. When linting, global rules apply to all content, and space-specific rules override global rules with the same `(rule_type, name)` key.

Five default rules are seeded on first migration:

| Rule | Type | Default Severity |
|------|------|-----------------|
| `avoid-simple` | terminology | warning |
| `avoid-just` | terminology | warning |
| `max-heading-depth` (H4) | formatting | warning |
| `max-sentence-length` (40 words) | formatting | info |
| `require-intro` | structure | warning |

**API endpoints:** See [API Reference — Style Rules Engine](api-reference.md#style-rules-engine) for full endpoint documentation.

There are no environment variables for the style rules engine — all limits are compile-time constants. Custom rules are created and managed entirely through the API.
