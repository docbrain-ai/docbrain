# DocBrain Configuration Reference

DocBrain uses a layered configuration system. Settings are resolved in this order (later = higher priority):

1. `{DOCBRAIN_CONFIG_DIR}/default.yaml` — committed base defaults
2. `{DOCBRAIN_CONFIG_DIR}/{APP_ENV}.yaml` — environment overrides (`APP_ENV=development|production`)
3. `{DOCBRAIN_CONFIG_DIR}/local.yaml` — gitignored developer overrides (optional)
4. **Environment variables** — always win; all existing env var names work unchanged

## Config File Location

| Variable | Default | Description |
|---|---|---|
| `DOCBRAIN_CONFIG_DIR` | `config` | Directory containing YAML config files. Override for Docker (`/etc/docbrain`), Kubernetes ConfigMap mounts, or custom bare-metal paths. |
| `APP_ENV` | `development` | Selects `{APP_ENV}.yaml`. Use `production` in production deployments. |

The server also accepts `--config-dir <path>` as a CLI argument (takes precedence over `DOCBRAIN_CONFIG_DIR`).

**Deployment examples:**
```bash
# Every start needs a fingerprint key: DOCBRAIN_FINGERPRINT_KEY, or a file named by
# DOCBRAIN_FINGERPRINT_KEY_FILE. Compose and Helm generate one; elsewhere generate it once with
# `openssl rand -base64 32`, keep it in .env or your secret store, and never change it casually.

# Bare metal — default (uses ./config/)
./docbrain-server

# Docker with mounted config dir
docker run -e DOCBRAIN_FINGERPRINT_KEY -e DOCBRAIN_CONFIG_DIR=/etc/docbrain -v /host/config:/etc/docbrain docbrain-server

# Kubernetes ConfigMap
# Mount ConfigMap at /etc/docbrain and set DOCBRAIN_CONFIG_DIR=/etc/docbrain

# Custom path via CLI arg
./docbrain-server --config-dir /opt/myapp/docbrain-config
```

## Infrastructure

| Variable | Required | Default | Description |
|---|---|---|---|
| `DATABASE_URL` | Yes | — | PostgreSQL connection string. e.g. `postgres://user:pass@host:5432/db` |
| `OPENSEARCH_URL` | Yes | `http://localhost:9200` | OpenSearch endpoint |
| `REDIS_URL` | No | `redis://localhost:6379` | Redis for working memory (session context). Optional — falls back to in-memory if not set. |
| `DOCBRAIN_FINGERPRINT_KEY` | Yes, or the file below | — | Key for the fingerprints DocBrain stores of secret values it finds (HMAC-SHA256). Docker Compose and the Helm chart generate one on first start when you supply none. See [The fingerprint key](#the-fingerprint-key) below. |
| `DOCBRAIN_FINGERPRINT_KEY_FILE` | No | — | Path to a file holding the key (what Compose's generated key uses). `DOCBRAIN_FINGERPRINT_KEY` wins when both are set. |
| `DB_MAX_CONNECTIONS` | No | `25` | Maximum connections in each PostgreSQL pool the server opens. It opens one pool per database role (at least two), so the database can see a multiple of this number. |
| `DB_CONNECT_TIMEOUT_SECS` | No | `10` | Seconds the server waits to open its database connections at start before giving up. |
| `DB_ACQUIRE_TIMEOUT_SECS` | No | `10` | Seconds a request waits for a free pooled connection before it fails. |
| `DB_IDLE_TIMEOUT_SECS` | No | `300` | Seconds an unused pooled connection is kept before it is closed. |

### The fingerprint key

DocBrain checks every capture for secrets (see
[What a capture never holds](agents.md#what-a-capture-never-holds)), and it has to remember what it found without
storing the secret itself. So it stores a keyed fingerprint (HMAC-SHA256 under this key) of:

- each secret value a capture was found to hold (a capture's findings);
- the text the language model judged in the secret check, so an unchanged text is not judged twice;
- each value an admin restored after a custom secret rule withdrew a capture (that value's exemption).

A plain hash of a short password can be guessed offline by anyone who holds the database or a backup. A keyed
fingerprint cannot be, without the key. That is also why the key is never stored in the database.

**Generated for you by default.** You do not have to do anything to start:

| Deployment | Where the key comes from | Where it lives |
|---|---|---|
| Docker Compose | The one-shot `keys` service writes 32 random bytes (base64, mode 600) on first start, unless you supplied a key or one exists. It never replaces a key and never prints one; a key file that exists but is empty or not a key stops the start with a message (restore it from backup, or delete it to generate a new one). | The `docbrain-keys` volume, file `fingerprint.key`, mounted read-only into the server (`DOCBRAIN_FINGERPRINT_KEY_FILE=/keys/fingerprint.key`). |
| Helm | Generated on first install (32 random bytes, base64) and looked up on every upgrade and reinstall, so it is never rotated (a rollback reapplies a stored manifest that carries the same value). | Its own Secret, `docbrain-fingerprint-key` for a release named `docbrain` (`<release>-docbrain-fingerprint-key` otherwise), key `DOCBRAIN_FINGERPRINT_KEY`, annotated `helm.sh/resource-policy: keep`: `helm uninstall` leaves it in place. Delete it by hand only to start fresh (findings reopen). The other credentials stay in the main Secret, which is not kept. |
| Anything else (bare metal, your own manifests) | You: `openssl rand -base64 32`. | `DOCBRAIN_FINGERPRINT_KEY`, or a file named by `DOCBRAIN_FINGERPRINT_KEY_FILE`. |

`helm template`, and GitOps tools that render charts offline (Argo CD, Flux with offline rendering), cannot look up
the existing Secret, so they render a **new** key on every run. There, always supply the key: `--set
fingerprintKey=...` from your secret store, or `fingerprintKeySecret.existingSecret` naming a Secret you manage.

**Supplying your own.** Set `DOCBRAIN_FINGERPRINT_KEY` (in `.env` for Compose, `fingerprintKey`, or
`fingerprintKeySecret.existingSecret` + `fingerprintKeySecret.key` for Helm), or point `DOCBRAIN_FINGERPRINT_KEY_FILE` at a file holding it. At least 32 random bytes, as 64+ hex
characters (an optional `0x` prefix) or standard base64:

```bash
openssl rand -base64 32     # or: openssl rand -hex 32
```

When both are set, `DOCBRAIN_FINGERPRINT_KEY` wins and the file is not read. A value set but invalid (including one
that is not valid UTF-8) is refused; it never falls back to the file.

Supplying your own key after one was generated (or replacing a supplied one) is a key change: see **Keep it stable**
below.

**Where Helm holds it.** A key supplied with `--set fingerprintKey=...` (or a values file) is stored by Helm in the
release's values (`helm get values`) and its manifest; a generated key is in the stored release manifest. Both are
Secrets in the release namespace, readable by whoever can read Secrets there. To keep the key out of Helm's release
records, create the Secret yourself and set `fingerprintKeySecret.existingSecret`.

**Required at boot.** When neither yields a valid key the server prints the refusal (to standard error, and as an
error line in its log) and exits with status 1, before it opens the database:

```text
DOCBRAIN_FINGERPRINT_KEY is not set (nor DOCBRAIN_FINGERPRINT_KEY_FILE): the server needs it to fingerprint secret findings; expected at least 64 hex characters (`openssl rand -hex 32`) or standard base64 of at least 32 bytes (`openssl rand -base64 32`), generated at random, never typed
```

A value or file that is too short, not hex or standard base64 (URL-safe base64 and missing padding are refused),
or that looks typed rather than generated (too few distinct characters, a repeated pattern, a placeholder) is
refused the same way, naming the setting (and the file's path) and the accepted shapes, never the value. A key file
that is missing, unreadable, empty or larger than 4 KiB is refused too.

**Never shown.** The key is read from the environment or the key file only. It is not part of the configuration
files, and no log line, error, health or config endpoint, or console page carries it. The boot log says only where
it came from (`fingerprint key loaded source=Env` or `source=File`).

**Back it up, separately from the database backups.** Back up the `docbrain-keys` volume (Compose) or the
fingerprint-key Secret (Helm) on the same schedule as the database, but not in the same place: a database backup stored together
with its key gives the reader back exactly what the key protects. Restoring a database without its key behaves like
a key change.

**Keep it stable.** Changing the key — rotating it, deleting the `docbrain-keys` volume (`docker compose down -v`),
removing a key you had supplied so a new one is generated, supplying your own key after one was generated, deleting
the chart's kept fingerprint-key Secret — makes
every stored fingerprint stop matching:

- a finding an admin restored or declined opens again, and waits for a decision again;
- a restored value's exemption stops matching, so that value is masked again in what DocBrain serves;
- texts the model already judged are judged again (more model calls, once).

Nothing is lost or exposed by a change; decisions have to be made again. There is no built-in rotation. All replicas
of one deployment must use the same key.

## LLM Provider

Set `LLM_PROVIDER` to choose your LLM backend. Supported providers: `anthropic`, `openai`, `bedrock`, `ollama`, `groq`, `openrouter`, `together`, `deepseek`, `mistral`, `xai`, `gemini`, `azure_openai`, `cohere`.

| Variable | Provider | Description |
|---|---|---|
| `LLM_PROVIDER` | All | Which LLM to use (see list above) |
| `LLM_MODEL_ID` | All | Model identifier (e.g. `claude-sonnet-4-5-20250929`, `gpt-4o`). Defaults to a provider-appropriate model if not set. |
| `LLM_TIMEOUT_SECS` | All except ollama | HTTP timeout in seconds for the LLM client (default: `120`). Applies to anthropic, openai (and its OpenAI-compatible siblings: groq, openrouter, together, deepseek, mistral, xai, gemini), azure_openai and cohere. Ollama has its own `OLLAMA_TIMEOUT_SECS` below, which takes precedence for that provider. Must be a positive integer — `0` fails config load, since a zero-second timeout would fail every call. |
| `ANTHROPIC_API_KEY` | anthropic | Your Anthropic API key |
| `OPENAI_API_KEY` | openai | Your OpenAI API key |
| `AWS_REGION` | bedrock | AWS region for Bedrock (e.g. `us-east-1`) |
| `AWS_ACCESS_KEY_ID` | bedrock | AWS access key (optional — see credential chain below) |
| `AWS_SECRET_ACCESS_KEY` | bedrock | AWS secret key (optional — see credential chain below) |

> **AWS Credential Chain**: Bedrock uses the AWS SDK default credential chain: env vars → `~/.aws/credentials` → IRSA (EKS) → EC2 Instance Profile → ECS Task Role. In production, use IRSA or instance profiles — no keys in env. Set `serviceAccount.create=true` and `serviceAccount.annotations.eks.amazonaws.com/role-arn` in Helm. The IAM role needs `bedrock:InvokeModel` and `bedrock:InvokeModelWithResponseStream` permissions.
| `OLLAMA_BASE_URL` | ollama | Ollama server URL (e.g. `http://localhost:11434`) |
| `OLLAMA_TIMEOUT_SECS` | ollama | HTTP timeout in seconds (default: `120`). Increase for large/slow models (e.g. 70B) to avoid "error decoding response body" when the model takes longer than 2 minutes. Example: `300` or `600`. |
| `OLLAMA_TLS_VERIFY` | ollama | Set `false` to skip TLS verification (default: `false`) |
| `FAST_MODEL_ID` | All | Fast/cheap model for background side-calls: intent classification, query rewriting, entity extraction. Falls back to `LLM_MODEL_ID` if not set. Recommended: Haiku (Bedrock/Anthropic), `gpt-4o-mini` (OpenAI), `llama-3.1-8b-instant` (Groq). Alias: `HAIKU_MODEL_ID` (deprecated). |
| `INGEST_LLM_MODEL_ID` | All | Model used **during ingest only** for image extraction. Falls back to `LLM_MODEL_ID` if not set. **Use a cheaper model here** — image extraction fires for every page with images and does not benefit from a powerful model. Example: use `LLM_MODEL_ID=claude-opus-4` for Q&A and `INGEST_LLM_MODEL_ID=us.anthropic.claude-haiku-4-5-20251001-v1:0` for ingest. Not setting this when using Opus 4 + extended thinking will cause throttling errors during ingest. |
| `DRAFT_MODEL_ID` | All | Model used for **autopilot draft generation** (two-phase reasoning + writing). Falls back to `LLM_MODEL_ID` if not set. Use a high-capability model here — drafts benefit from stronger reasoning. Example: `claude-opus-4` or `gpt-4o`. |
| `DRAFT_LLM_PROVIDER` | All | Provider for draft generation. Falls back to `LLM_PROVIDER` if not set. Allows cross-provider drafting — e.g. use Gemini Flash for Q&A but Anthropic Claude for drafts. When set, DocBrain creates a separate LLM client for draft generation. |
| `LLM_THINKING_BUDGET` | anthropic/bedrock | Extended thinking token budget (tokens). `0` or unset = disabled. Only applies to the primary `LLM_MODEL_ID` — never to `FAST_MODEL_ID` or `INGEST_LLM_MODEL_ID`. |
| `OPENAI_BASE_URL` | openai | Base URL of an OpenAI-compatible endpoint to use in place of OpenAI's own, for a proxy such as LiteLLM, vLLM or LocalAI. Unset (the default) calls OpenAI. |
| `GROQ_API_KEY` | groq | Your Groq API key. Unset (the default) sends an empty key, so every call fails until it is set. |
| `OPENROUTER_API_KEY` | openrouter | Your OpenRouter API key. Unset (the default) sends an empty key, so every call fails until it is set. |
| `TOGETHER_API_KEY` | together | Your Together AI API key. Unset (the default) sends an empty key, so every call fails until it is set. |
| `DEEPSEEK_API_KEY` | deepseek | Your DeepSeek API key. Unset (the default) sends an empty key, so every call fails until it is set. |
| `MISTRAL_API_KEY` | mistral | Your Mistral API key. Unset (the default) sends an empty key, so every call fails until it is set. |
| `XAI_API_KEY` | xai | Your xAI API key. Unset (the default) sends an empty key, so every call fails until it is set. |
| `GEMINI_API_KEY` | gemini | Your Google Gemini API key. Unset (the default) sends an empty key, so every call fails until it is set. |
| `HAIKU_MODEL_ID` | All | Deprecated name for `FAST_MODEL_ID`. The server and ingest read settings through the config loader, where both names write the same setting and `HAIKU_MODEL_ID` is applied after `FAST_MODEL_ID`, so when both are set `HAIKU_MODEL_ID` wins. Only code that builds the client straight from the environment reads `FAST_MODEL_ID` first and uses this name only when it is unset. |
| `DOCBRAIN_LLM_EFFORT` | bedrock | How hard Anthropic models on Bedrock reason: `low`, `medium`, `high`, `xhigh` or `max`. Unset, or a value that is not one of those (which logs a warning), uses `high`. The higher settings mostly add latency. |
| `OLLAMA_VISION_ENABLED` | ollama | Set `false` or `0` to stop sending images to Ollama for description. Anything else, or unset, leaves vision on (the default is on). |

### OpenAI-compatible providers

The following providers use the OpenAI Chat Completions API format. Set `LLM_PROVIDER` to the provider name and the corresponding API key:

| Provider | `LLM_PROVIDER` | API Key Variable | Default `LLM_MODEL_ID` |
|---|---|---|---|
| OpenAI | `openai` | `OPENAI_API_KEY` | `gpt-4o` |
| Groq | `groq` | `GROQ_API_KEY` | `llama-3.3-70b-versatile` |
| OpenRouter | `openrouter` | `OPENROUTER_API_KEY` | `openai/gpt-4o` |
| Together AI | `together` | `TOGETHER_API_KEY` | `meta-llama/Llama-3.3-70B-Instruct-Turbo` |
| DeepSeek | `deepseek` | `DEEPSEEK_API_KEY` | `deepseek-chat` |
| Mistral | `mistral` | `MISTRAL_API_KEY` | `mistral-small-latest` |
| xAI (Grok) | `xai` | `XAI_API_KEY` | `grok-3` |
| Google Gemini | `gemini` | `GEMINI_API_KEY` | `gemini-2.5-flash` |

You can also use `OPENAI_BASE_URL` with `LLM_PROVIDER=openai` to point at any OpenAI-compatible proxy (e.g. LiteLLM, vLLM, LocalAI).

### Azure OpenAI

| Variable | Required | Description |
|---|---|---|
| `AZURE_OPENAI_API_KEY` | Yes | Azure OpenAI API key |
| `AZURE_OPENAI_ENDPOINT` | Yes | Resource endpoint. e.g. `https://my-resource.openai.azure.com` |
| `AZURE_OPENAI_API_VERSION` | No | API version. Default: `2024-02-01` |
| `LLM_MODEL_ID` | Yes | Deployment name (e.g. `gpt-4o`) — must match your Azure deployment |

Example:
```bash
LLM_PROVIDER=azure_openai
AZURE_OPENAI_API_KEY=your-key
AZURE_OPENAI_ENDPOINT=https://my-resource.openai.azure.com
LLM_MODEL_ID=gpt-4o           # your deployment name
```

### Cohere

| Variable | Required | Description |
|---|---|---|
| `COHERE_API_KEY` | Yes | Cohere API key |
| `LLM_MODEL_ID` | No | Default: `command-r-plus` |

### Vertex AI

Vertex AI lets you run Gemini (and other hosted models) through Google Cloud with enterprise security, VPC controls, and no data leaving your GCP project.

| Variable | Required | Description |
|---|---|---|
| `VERTEX_PROJECT` | Yes | GCP project ID (e.g. `my-project-123`) |
| `VERTEX_REGION` | No | GCP region. Default: `us-central1` |
| `LLM_MODEL_ID` | No | Model ID. Default: `google/gemini-2.5-flash` |

Authentication uses the GCP credential chain in order:
1. `GOOGLE_APPLICATION_CREDENTIALS` env var pointing to a service account JSON file
2. Application Default Credentials (`gcloud auth application-default login`)
3. GKE Workload Identity (attach a KSA to a GSA with `roles/aiplatform.user`)
4. GCE metadata server (Compute Engine / Cloud Run default service account)

Example:
```bash
LLM_PROVIDER=vertex_ai
VERTEX_PROJECT=my-project-123
VERTEX_REGION=us-central1
LLM_MODEL_ID=google/gemini-2.5-flash
# No API key needed — uses GCP credential chain
```

For Kubernetes/GKE with Workload Identity:
```yaml
serviceAccount:
  annotations:
    iam.gke.io/workload-identity-pool: "my-project-123.svc.id.goog"
    iam.gke.io/service-account: "docbrain@my-project-123.iam.gserviceaccount.com"
```

> **Note:** Vertex AI support requires the `vertex` feature flag. The Docker image and Helm chart include this feature by default.

## Embedding Provider

Set `EMBED_PROVIDER` to choose your embedding model. One of: `openai`, `bedrock`, `ollama`.

| Variable | Provider | Description |
|---|---|---|
| `EMBED_PROVIDER` | All | Which embedding model to use: `openai`, `bedrock`, `ollama` |
| `EMBED_MODEL_ID` | All | Embedding model identifier (e.g. `text-embedding-3-small`, `cohere.embed-v4:0`) |

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

### Embedding provider tuning

Each setting applies only while its provider is the one selected.

| Variable | Default | Description |
|---|---|---|
| `BEDROCK_INITIAL_CONCURRENCY` | `2` | Bedrock: concurrent embedding requests the adaptive limiter starts at. It backs off when Bedrock throttles. Held between 1 and `BEDROCK_MAX_CONCURRENCY`. |
| `BEDROCK_MAX_CONCURRENCY` | `16` | Bedrock: the most concurrent embedding requests the limiter will allow. |
| `OLLAMA_EMBED_MAX_RETRIES` | `5` | Ollama: how many times one embedding call is tried before it fails, with a growing pause between tries (capped at 10 seconds). Zero or a value that is not a number keeps the default. |

### Learning pipeline

Settings for the fine-tuned embedding model the `docbrain-trainer` sidecar produces (`EMBED_PROVIDER=local`). Several are accepted but not yet acted on (the rows say which), and the server warns at start when one of the three promotion settings is set.

| Variable | Default | Description |
|---|---|---|
| `EMBED_LOCAL_INFERENCE_URL` | — | Base URL of the `docbrain-trainer` inference sidecar. Required when `EMBED_PROVIDER=local`; the provider is refused without it. |
| `LEARNING_AZURE_ACCOUNT` | — | Azure storage account for model storage. Required when the storage backend is `azure`; model storage cannot be set up without it. |
| `LEARNING_MIN_FEEDBACK_USERS` | `3` | Fewest distinct identified users whose feedback a training run needs. Anonymous feedback does not count, and a run with fewer is rejected by the poison guard. |
| `LEARNING_ENABLED` | `false` | Turns the learning pipeline on. Off, the configured embedding provider is used unchanged and none of the other `LEARNING_*` settings matter. |
| `LEARNING_STORAGE_BACKEND` | `local` | Where trained model files are kept: `local`, `s3`, `gcs` or `azure`. Any other value stops the pipeline from starting. |
| `LEARNING_LOCAL_PATH` | `/app/models` | Directory for model files when the backend is `local`. |
| `LEARNING_S3_BUCKET` | — | S3 bucket for model files. Required when the backend is `s3`; the pipeline cannot start without it. |
| `LEARNING_S3_PREFIX` | `docbrain/models` | Key prefix inside the S3 bucket. |
| `LEARNING_S3_REGION` | `us-east-1` | AWS region of the S3 bucket. |
| `LEARNING_GCS_BUCKET` | — | Google Cloud Storage bucket for model files. Required when the backend is `gcs`; the pipeline cannot start without it. |
| `LEARNING_GCS_PREFIX` | `docbrain/models` | Object prefix inside the GCS bucket. |
| `LEARNING_AZURE_CONTAINER` | — | Azure Blob container for model files. Required, together with `LEARNING_AZURE_ACCOUNT`, when the backend is `azure`. |
| `LEARNING_TRAINER_URL` | `http://docbrain-trainer:8080` | Base URL of the `docbrain-trainer` sidecar the pipeline sends training work to. |
| `LEARNING_BASE_MODEL_ID` | `sentence-transformers/all-MiniLM-L6-v2` | Hugging Face model that fine-tuning starts from. It must match the base model the trainer sidecar is configured with. |
| `LEARNING_MIN_TRIPLETS` | `200` | Fewest training triplets (a query, a right passage and a wrong one) a run needs. With fewer, the run is skipped and logged. |
| `LEARNING_CIRCUIT_BREAKER_THRESHOLD` | `0.05` | How far, as a fraction, a new model may lower average answer confidence before it is rolled back. `0.05` is 5 percent. |
| `LEARNING_POISON_DETECTION` | `true` | Reject a training run whose feedback looks like it came from bots or one person. `false` skips the check. |
| `LEARNING_MAX_SINGLE_USER_FRACTION` | `0.80` | Largest share of a run's feedback one user may supply before the poison check rejects the run. |
| `TRAINER_API_KEY` | — | Key the server sends to the `docbrain-trainer` sidecar in the `X-Trainer-Key` header when it asks for embeddings (`EMBED_PROVIDER=local`). Unset sends no header, which suits a sidecar that has no key. |

## Document Sources

A source is enabled by the **presence of its block** under the top-level
`sources:` key in `config/default.yaml` or `config/local.yaml` — there is no
env var that selects a source.

> **`SOURCE_TYPE` no longer exists.** Earlier versions selected the source with
> `SOURCE_TYPE=local|confluence|github`. It is gone from the code; setting it
> has no effect. If you set it and nothing else, ingest falls back to the
> `LOCAL_DOCS_PATH` default (`./examples/sample-docs`) and DocBrain answers from
> the sample documents while appearing to work. Enable the `sources:` block
> instead — see [Ingestion](ingestion.md) and the quickstart.

Resource lists (spaces, repos, projects, channels) are always explicit; an empty
list is a startup error, never "ingest everything the token can see".

```yaml
sources:
  local:
    path: /path/to/docs        # or set LOCAL_DOCS_PATH
  confluence:
    base_url: https://your-org.atlassian.net/wiki
    user_email: ${CONFLUENCE_USER_EMAIL}
    api_token: ${CONFLUENCE_API_TOKEN}
    space_keys: [DOCS, ENG]
```

The scalar credentials below are still read from the environment; only the
source selection and the resource lists moved to YAML.

### Local Files

| Variable | Default | Description |
|---|---|---|
| `LOCAL_DOCS_PATH` | `./examples/sample-docs` | Path to local markdown/text files to ingest |
| `LOCAL_ROOT_HOLDS_REPOSITORIES` | `false` | Set `true` when the path holds several repositories, one per top-level directory, rather than one tree. Claim checking is then confined to one repository at a time. DocBrain cannot tell the two layouts apart, so it has to be told. |

### Confluence

| Variable | Default | Description |
|---|---|---|
| `CONFLUENCE_BASE_URL` | — | Full Confluence base URL. e.g. `https://your-org.atlassian.net/wiki` |
| `CONFLUENCE_USER_EMAIL` | — | Email of the Confluence user (Cloud only — not used for Data Center) |
| `CONFLUENCE_API_TOKEN` | — | API token (Cloud) or Personal Access Token (Data Center) |
| `CONFLUENCE_SPACE_KEYS` | — | Comma-separated space keys to ingest. e.g. `DOCS,ENG,PLATFORM`. Optional when `CONFLUENCE_PAGE_IDS` is set. |
| `CONFLUENCE_PAGE_IDS` | — | Comma-separated page IDs to ingest, **including all descendants**. Pages can be from different spaces. e.g. `421856743,41912006`. Can be used alone or together with `CONFLUENCE_SPACE_KEYS`. |
| `CONFLUENCE_API_VERSION` | `v2` | `v2` for Confluence Cloud, `v1` for self-hosted Data Center |
| `CONFLUENCE_PAGE_LIMIT` | `0` | Max pages to ingest per space; `0` (the default) means no limit, so the knob does nothing until set |
| `CONFLUENCE_TLS_VERIFY` | `true` | Set to `false` if using self-signed or internal CA certificates |
| `CONFLUENCE_LICENSED_GROUPS` | — | Comma-separated names of extra Confluence groups whose members hold a licence. `confluence-users` and `confluence-users-<site>` are always counted; add a group here when licences come through a custom one, or those people are denied. |
| `CONFLUENCE_WEBHOOK_SECRET` | — | Shared secret that signs Confluence webhook calls to `/confluence/events`. Setting it turns the Confluence webhook on; it also needs `CONFLUENCE_BASE_URL` and `CONFLUENCE_API_TOKEN`, or the server logs a warning and leaves it off. |

**Ingestion modes:**

| Mode | Config | Behaviour |
|------|--------|-----------|
| Whole spaces | `CONFLUENCE_SPACE_KEYS=DOCS,ENG` | All pages in the listed spaces |
| Specific page trees | `CONFLUENCE_PAGE_IDS=421856743,41912006` | Listed pages + every descendant, from any space |
| Both | `CONFLUENCE_SPACE_KEYS=DOCS` + `CONFLUENCE_PAGE_IDS=421856743` | Spaces first, then any page trees not already covered |

### Confluence background syncs

Two server-side jobs that need the Confluence connection above. They stay idle when Confluence is not configured.

| Variable | Default | Description |
|---|---|---|
| `CONFLUENCE_USER_SYNC_INTERVAL_SECS` | `900` | Seconds between runs of the job that resolves Confluence account ids on documents to a name and email. `0` turns it off. |
| `CONFLUENCE_USER_SYNC_BATCH_SIZE` | `100` | Account ids resolved per run. |
| `CONFLUENCE_USER_SYNC_REFRESH_AFTER_DAYS` | `30` | Days before an account id that was already resolved is looked up again. |
| `CONFLUENCE_USER_SYNC_PER_CALL_DELAY_MS` | `100` | Milliseconds between Confluence calls in that job; 100 holds it to about 10 requests a second. |
| `CONFLUENCE_SPACE_ACCESS_SYNC_INTERVAL_SECS` | `0` | Seconds between recomputes of who can see each space. `0` leaves it off. It writes access decisions, so turn it on deliberately after checking coverage. |

### GitHub PR Reviews (`INGEST_SOURCES=github_pr`)

Ingests merged pull request descriptions and review comments as searchable documents.
This captures the architectural "why" that lives in code review threads.

| Variable | Default | Description |
|---|---|---|
| `GITHUB_PR_TOKEN` | — | GitHub personal access token with `repo:read` (or `public_repo`) scope |
| `GITHUB_PR_REPO` | — | Repository in `owner/repo` format. e.g. `acme/platform` |
| `GITHUB_PR_MIN_COMMENTS` | `1` | Minimum total review comments required to index a PR. Set higher (e.g. `3`) to skip trivial PRs. |
| `GITHUB_PR_LABELS` | — | Comma-separated label filter. Only PRs with at least one matching label are indexed. Empty = index all. |
| `GITHUB_PR_API_URL` | `https://api.github.com` | Override for GitHub Enterprise deployments |

**Notes:**
- How far back to look is `lookback_days` under `sources.github.pull_requests` in the config file (default `365`); it has no environment variable.
- Only **merged** PRs are indexed — open, closed-without-merge, and draft PRs are skipped.
- Bot comments (Dependabot, GitHub Actions, Codecov, etc.) are automatically filtered out.
- Deleted review comments (empty body) are excluded.
- Very large PR comment threads (100+ comments per PR) are paginated automatically.
- **Cross-document references** are automatically extracted from PR body and review comments — URLs to Jira tickets, other PRs, GitLab MRs, Confluence pages, and Slack threads are classified and stored in the reference graph.
- GitHub API rate limit is 5000 req/hr for authenticated tokens. A full initial sync of a large repo may approach this limit; subsequent syncs only fetch updated PRs.

### GitHub Capture Webhook Security (`INGEST_SOURCES=github_pr` + real-time capture)

The GitHub capture feature lets engineers trigger ingestion by commenting `@docbrain capture` on any PR or issue. These optional variables restrict which repos and users can trigger captures.

| Variable | Default | Description |
|---|---|---|
| `GITHUB_CAPTURE_ALLOWED_REPOS` | — | Comma-separated `owner/repo` pairs that are allowed to trigger capture. Empty = all repos. e.g. `myorg/backend,myorg/frontend` |
| `GITHUB_CAPTURE_ALLOWED_USERS` | — | Comma-separated GitHub usernames allowed to trigger capture. Empty = all users. e.g. `alice,bob` |
| `GITHUB_WEBHOOK_SECRET` | — | Secret that signs GitHub webhook calls for `@docbrain capture`. Capture stays off unless this and a bot token (`GITHUB_BOT_TOKEN`, or `GITHUB_PR_TOKEN` as a fallback) are both set. |
| `GITHUB_BOT_TOKEN` | — | GitHub token the capture feature uses to read threads and post replies. Also the second choice, after `GITHUB_TOKEN`, when DocBrain pulls a `.docbrain/style.md` policy file. |

A 500KB content size guard is applied to all capture requests. If a PR or issue thread exceeds this limit, DocBrain posts a reply explaining that the thread was too large to capture.

---

### GitLab MR + Review Notes (`INGEST_SOURCES=gitlab_mr`)

Ingests merged merge request descriptions and discussion threads. Works with gitlab.com and self-hosted GitLab instances.

| Variable | Default | Description |
|---|---|---|
| `GITLAB_MR_TOKEN` | — | GitLab personal or project access token with `api` scope |
| `GITLAB_MR_BASE_URL` | `https://gitlab.com` | Base URL for self-hosted GitLab (e.g. `https://gitlab.mycompany.com`) |
| `GITLAB_MR_PROJECT_IDS` | — | Comma-separated project paths or IDs. e.g. `acme/platform,acme/backend` |
| `GITLAB_MR_MIN_NOTES` | `1` | Minimum discussion notes to index an MR. Set higher to skip trivial MRs. |
| `GITLAB_MR_LABELS` | — | Comma-separated label filter. Empty = index all. |
| `GITLAB_MR_TLS_VERIFY` | `true` | Set to `false` to skip TLS certificate verification for self-hosted with internal CA |
| `GITLAB_TOKEN` | — | GitLab token with `api` scope for the GitLab source. Also the fallback token when a policy-file pull has none of its own. |
| `GITLAB_BASE_URL` | `https://gitlab.com` | Base URL of the GitLab instance for the GitLab source. Set it for a self-hosted GitLab. |
| `GITLAB_TLS_VERIFY` | `true` | Set `false` to skip TLS certificate checks for a self-hosted GitLab with a self-signed certificate. |

**Notes:**
- How far back to look is `lookback_days` under `sources.gitlab.merge_requests` in the config file (default `365`); it has no environment variable.
- Only **merged** MRs are indexed — open, draft (`Draft:` / `WIP:` title prefix or `draft: true` field), and unmerged MRs are skipped.
- System notes (label changes, approvals, "merged" events) are automatically filtered — only human discussion notes are indexed.
- **Cross-document references** are automatically extracted from MR descriptions and notes — URLs to GitHub PRs, Jira tickets, Confluence pages, and other GitLab MRs/issues are classified. GitLab shorthand references (`!123` for MRs, `#123` for issues) within the same project are also resolved.
- Pagination uses the `X-Next-Page` response header pattern.
- GitLab.com rate limit: 600 req/min. Self-hosted limits vary.

---

### GitLab MR Capture Webhook

The GitLab capture feature lets engineers trigger ingestion by commenting `@docbrain capture` on any merge request. When triggered, DocBrain fetches the full MR discussion (excluding system notes) and indexes it immediately.

**Setup:**

1. Generate a webhook secret:
   ```bash
   openssl rand -hex 32
   ```

2. Set environment variables:
   ```env
   GITLAB_CAPTURE_WEBHOOK_SECRET=your-webhook-secret
   GITLAB_CAPTURE_TOKEN=glpat-...         # Personal access token with api scope
   GITLAB_CAPTURE_BASE_URL=https://gitlab.com   # Default; set for self-hosted instances
   GITLAB_CAPTURE_ALLOWED_USERS=alice,bob       # Optional — empty = all users
   GITLAB_CAPTURE_ALLOWED_PROJECTS=myorg/myrepo # Optional — empty = all projects
   ```

3. Register the webhook in GitLab: **Project → Settings → Webhooks**
   - URL: `https://your-docbrain-host/api/v1/gitlab/events`
   - Secret token: same value as `GITLAB_CAPTURE_WEBHOOK_SECRET`
   - Trigger: enable **Comments**
   - Save webhook

**Behavior:**
- DocBrain only responds to comments containing `@docbrain capture`
- All MR notes are fetched (system notes like merge bot events are excluded)
- A 500KB content size guard is applied — threads exceeding this limit are skipped silently (no reply)
- On success, DocBrain posts a reply note: `✅ Captured by DocBrain — N chunks indexed and immediately searchable.\nThis MR will feed Autopilot's next gap analysis run.`
- On failure, DocBrain posts: `⚠️ Capture failed: <error>`
- Reply notes are only posted when `GITLAB_CAPTURE_TOKEN` is set
- User and project allowlists prevent unauthorized users or repos from triggering capture

| Variable | Default | Description |
|---|---|---|
| `GITLAB_CAPTURE_WEBHOOK_SECRET` | — | HMAC secret shared with GitLab for webhook signature verification |
| `GITLAB_CAPTURE_TOKEN` | — | GitLab personal access token with `api` scope, used to fetch MR notes and post reply comments |
| `GITLAB_CAPTURE_BASE_URL` | `https://gitlab.com` | GitLab instance base URL (override for self-hosted) |
| `GITLAB_CAPTURE_ALLOWED_USERS` | — | Comma-separated GitLab usernames allowed to trigger capture. Empty = all users. |
| `GITLAB_CAPTURE_ALLOWED_PROJECTS` | — | Comma-separated project paths allowed to trigger capture. Empty = all projects. e.g. `myorg/myrepo` |
| `GITLAB_BOT_USER_ID` | — | Numeric GitLab user id of the account behind `GITLAB_CAPTURE_TOKEN`. Events authored by this user are ignored, so the bot does not react to its own notes. A value that is not a number is ignored. |
| `GITLAB_CAPTURE_TLS_INSECURE` | `false` | `true`, `1` or `yes` skips TLS certificate checks when calling the capture GitLab instance. Use it only for a self-signed certificate you cannot replace. |

---

### Linear (`INGEST_SOURCES=linear`)

Ingests completed and cancelled Linear issues. Linear uses Markdown natively — no content conversion needed.

| Variable | Default | Description |
|---|---|---|
| `LINEAR_API_KEY` | — | Linear personal API key — set in `config/local.yaml` or env var |
| `LINEAR_TEAMS` | — | Comma-separated team keys to index (e.g. `ENG,OPS`). Empty = all teams. |
| `LINEAR_LOOKBACK_DAYS` | `365` | How many days back to ingest completed/cancelled issues |
| `LINEAR_STATES` | `Done,Cancelled,Duplicate` | Comma-separated state names to include |

**Notes:**
- Only issues in completed-type states are indexed. Open/in-progress issues are skipped.
- Comments are included in the document content with author attribution.
- Sub-issues are not fetched for v1 — only top-level issues are indexed.
- Linear API rate limit: 1500 req/hr for personal API keys.

---

### PagerDuty (`INGEST_SOURCES=pagerduty`)

Ingests resolved incidents as postmortem mini-documents, including a chronological timeline of human actions (notifications, escalations, acknowledgements, annotations, resolutions).

| Variable | Default | Description |
|---|---|---|
| `PAGERDUTY_API_TOKEN` | — | PagerDuty REST v2 API token — set in `config/local.yaml` or env var |
| `PAGERDUTY_LOOKBACK_DAYS` | `180` | Days back to ingest resolved incidents |
| `PAGERDUTY_MIN_DURATION` | `5` | Skip incidents shorter than this many minutes (filters noise/blips) |
| `PAGERDUTY_SERVICES` | — | Comma-separated PagerDuty service IDs to filter. Empty = all services. |

**`config/local.yaml` example:**
```yaml
pagerduty:
  api_token: u+XXXXXXXX
  lookback_days: 365
  min_duration_minutes: 10
  # services: SVC001,SVC002   # optional service filter
```

**Notes:**
- Machine log entries (webhook deliveries, reach_trigger) are filtered — only human actions are indexed.
- Incidents with no timeline (auto-resolved) ingest description only.
- Metadata includes: `severity`, `urgency`, `service`, `duration_minutes`, `teams`, `escalated`.
- Rate limit: 960 requests/minute — well within ingest budget.

---

### OpsGenie (`INGEST_SOURCES=opsgenie`)

Ingests closed OpsGenie alerts with full activity logs as postmortem documents.

| Variable | Default | Description |
|---|---|---|
| `OPSGENIE_API_KEY` | — | OpsGenie API key — set in `config/local.yaml` or env var |
| `OPSGENIE_LOOKBACK_DAYS` | `180` | Days back to ingest closed alerts |
| `OPSGENIE_MIN_DURATION` | `5` | Skip alerts shorter than this many minutes |
| `OPSGENIE_TEAMS` | — | Comma-separated team names to filter. Empty = all teams. |

**`config/local.yaml` example:**
```yaml
opsgenie:
  api_key: XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX
  lookback_days: 365
  min_duration_minutes: 10
  # teams: platform,payments   # optional team filter
```

**Notes:**
- Priority P1–P5 is normalized to `critical/high/medium/low/informational` in metadata.
- Alert logs are fetched per alert; `closedAt` timestamp is used to compute duration.
- Metadata includes: `severity`, `priority`, `source`, `duration_minutes`, `tags`, `teams`.

---

### Zendesk (`INGEST_SOURCES=zendesk`)

Ingests solved support tickets with full public conversation threads.

| Variable | Default | Description |
|---|---|---|
| `ZENDESK_SUBDOMAIN` | — | Zendesk subdomain — e.g. `acme` for `acme.zendesk.com` |
| `ZENDESK_EMAIL` | — | Agent email address used for Basic Auth |
| `ZENDESK_API_TOKEN` | — | Zendesk API token — set in `config/local.yaml` or env var |
| `ZENDESK_LOOKBACK_DAYS` | `180` | Days back to ingest solved tickets |
| `ZENDESK_MIN_RATING` | — | Satisfaction filter: `good`, `bad`, or empty (include all) |

**`config/local.yaml` example:**
```yaml
zendesk:
  subdomain: acme
  email: support-bot@acme.com
  api_token: your-zendesk-api-token
  lookback_days: 365
  # min_rating: good   # optional — only "good" satisfaction tickets
```

**Notes:**
- Uses Zendesk Search API with `type:ticket+status:solved+created>DATE` query.
- Internal notes (`public: false` comments) are excluded.
- Agent email signatures are stripped (lines starting with `--`, `Best regards,`, etc.).
- HTML in comment bodies is converted to plain text; code blocks become Markdown fences.
- Tickets with fewer than 2 public comments are skipped.
- Metadata includes: `ticket_id`, `priority`, `requester_id`, `tags`, `satisfaction_score`.

---

### Intercom (`INGEST_SOURCES=intercom`)

Ingests resolved customer conversations with full message threads.

| Variable | Default | Description |
|---|---|---|
| `INTERCOM_ACCESS_TOKEN` | — | Intercom OAuth access token — set in `config/local.yaml` or env var |
| `INTERCOM_LOOKBACK_DAYS` | `180` | Days back to ingest conversations |
| `INTERCOM_MIN_MESSAGES` | `2` | Skip conversations with fewer than this many messages |
| `INTERCOM_TAGS` | — | Comma-separated tag names to filter. Empty = all conversations. |

**`config/local.yaml` example:**
```yaml
intercom:
  access_token: dG9rOjxxxxxxxxxxxxxxxx
  lookback_days: 365
  min_messages: 3
  # tags: billing,enterprise   # optional tag filter
```

**Notes:**
- Uses Intercom Conversations API with cursor pagination (`starting_after`).
- Internal notes (`part_type: note`) are excluded from the indexed content.
- HTML is stripped; Intercom `display_as=plaintext` mode is requested.
- Conversations are truncated to 80 messages max.
- Metadata includes: `tags`.

---

### Microsoft Teams (`INGEST_SOURCES=ms_teams`)

Ingests channel message threads, 1:1/group chats, channel files (SharePoint), and meeting transcripts from Microsoft Teams via the Microsoft Graph API.

| Variable | Default | Description |
|---|---|---|
| `MS_TEAMS_TENANT_ID` | — | Azure AD / Entra tenant ID |
| `MS_TEAMS_CLIENT_ID` | — | Entra app registration client ID |
| `MS_TEAMS_CLIENT_SECRET` | — | Client secret for client credentials OAuth flow |
| `MS_TEAMS_TEAMS` | — | Comma-separated team names or IDs to filter. Empty = all teams. e.g. `Engineering,Platform` |
| `MS_TEAMS_LOOKBACK_DAYS` | `90` | How many days back to ingest messages |
| `MS_TEAMS_INCLUDE_CHATS` | `false` | Include 1:1 and group chat messages. High volume — enable with care. |
| `MS_TEAMS_INCLUDE_FILES` | `true` | Include files shared in channels (from SharePoint) |
| `MS_TEAMS_INCLUDE_TRANSCRIPTS` | `true` | Include meeting transcripts (requires application access policy) |
| `MS_TEAMS_MIN_REPLIES` | `2` | Minimum replies for a channel message thread to be indexed |
| `MS_TEAMS_WEBHOOK_CLIENT_STATE` | — | The `clientState` value set on the Graph change-notification subscription. Live ingestion from Teams webhooks needs it together with the tenant, client and secret above, and a notification whose value differs is dropped. |

**`config/local.yaml` example:**
```yaml
ms_teams:
  tenant_id: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
  client_id: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
  client_secret: your-client-secret
  teams: Engineering,Platform
  lookback_days: 90
  include_chats: false
  include_files: true
  include_transcripts: true
  min_replies: 2
```

**Prerequisites:**

1. **Register an app** in [Azure Entra ID](https://portal.azure.com/#blade/Microsoft_AAD_RegisteredApps/ApplicationsListBlade) (formerly Azure AD)
2. **Add application permissions** (not delegated):
   - `Team.ReadBasic.All` — list teams
   - `Channel.ReadBasic.All` — list channels
   - `ChannelMessage.Read.All` — read channel messages
   - `Files.Read.All` — read channel files from SharePoint
   - `Chat.Read.All` — read 1:1/group chats (only needed if `include_chats: true`)
   - `OnlineMeetingTranscript.Read.All` — read meeting transcripts (only needed if `include_transcripts: true`)
3. **Tenant admin must grant consent** — click "Grant admin consent" in the Azure portal. This is the main onboarding friction point.
4. **For transcripts:** The tenant admin must also create an [application access policy](https://learn.microsoft.com/en-us/microsoftteams/teams-recording-policy) and assign it to the service account users whose meetings should be accessible.

**What gets ingested:**

| Data Type | Source ID Pattern | Content |
|---|---|---|
| Channel message threads | `msg:{team_id}:{channel_id}:{message_id}` | Threaded conversation in Markdown (like Slack threads) |
| 1:1/group chats | `chat:{chat_id}` | Full chat conversation |
| Channel files | `file:{drive_id}:{item_id}` | Text content of `.md`, `.txt`, `.csv`, `.json`, `.yaml`, `.xml`, `.html`, `.rst` files |
| Meeting transcripts | `transcript:{meeting_id}:{transcript_id}` | VTT converted to readable Markdown with speaker grouping |

**Notes:**
- Microsoft Graph API rate limit: 4 requests/second per app per team. DocBrain automatically throttles and respects `Retry-After` headers on 429 responses.
- **Cross-document references** are extracted from HTML message bodies (`<a href="url">` tags) before HTML stripping, so link URLs are preserved even when the link text differs from the URL. Bare URLs in plain-text messages are also extracted.
- Private channels have separate SharePoint site collections — file extraction handles this transparently.
- Team filtering is case-insensitive on display names and also matches by team ID.
- Binary files (PDF, DOCX, images) are skipped — only text-extractable file types are indexed.
- Meeting transcripts are only available if transcription was enabled during the meeting.
- Files larger than 10 MB are skipped.

---

### GitHub

| Variable | Default | Description |
|---|---|---|
| `GITHUB_TOKEN` | — | GitHub personal access token (for private repos) |
| `GITHUB_API_URL` | `https://api.github.com` | GitHub API base URL for code and pull-request ingest and for policy-file pulls. Set it for GitHub Enterprise. Shares its setting with `GITHUB_PR_API_URL`. |

The repository and branch to ingest are listed in `sources.github.code.repos` in the config file, as `owner/repo` or `owner/repo:branch`; there are no environment variables for them.

### Policy file sources (`.docbrain/style.md`)

A repository can carry a `.docbrain/style.md` that DocBrain pulls in as policy rules. These settings point the pullers at self-hosted hosts and set when the quality banner flags rule sprawl.

| Variable | Default | Description |
|---|---|---|
| `GITLAB_API_URL` | `https://gitlab.com` | Base URL for GitLab when pulling a policy file. Set it for a self-hosted GitLab. |
| `BITBUCKET_API_URL` | `https://api.bitbucket.org` | Base URL for Bitbucket Cloud when pulling a policy file. |
| `BITBUCKET_TOKEN` | — | Bearer token for Bitbucket policy pulls, used when the source has no token of its own. |
| `STYLE_SPRAWL_STALE_DAYS` | `90` | Days without an edit before a space override counts as stale in the sprawl banner on the quality page. Accepts 1 to 365; anything else keeps the default. |
| `STYLE_SPRAWL_EXCESS_THRESHOLD` | `20` | Overrides in one space above which the sprawl banner flags that space. Accepts 1 to 1000; anything else keeps the default. |
| `POLICY_FILE_SYNC_INTERVAL_SECS` | `900` | Seconds between pulls of registered policy files. `0` turns the sync off. A value that is not a whole number keeps the default. |

### Ingest runs

What an ingest run does around the sources, read by the ingest binary and the server.

| Variable | Default | Description |
|---|---|---|
| `INGEST_PRUNE_DELETED` | `true` | The code default is `false` when the `ingest:` block is absent; the shipped `config/default.yaml` has the block, so it is `true`. After a run, remove documents that have disappeared from their source (their chunks leave the index and the document is marked deleted). Skipped when the run saw zero documents, as a guard against a network failure wiping the corpus. |
| `INGEST_RUN_ON_STARTUP` | `false` | Run one ingest when the server starts, before the first scheduled run. |
| `INGEST_SKIP_EXTENSIONS` | — | Comma-separated file extensions to skip, without dots (`png,zip`). When set it replaces the built-in skip list (images, audio, video, archives and similar), and an empty value skips nothing. Unset keeps the built-in list. |
| `FORCE_REINGEST` | `false` | `true` or `1` re-embeds every document whatever its stored version. Set it for one run, then remove it. |
| `DOCBRAIN_INGEST_BIN` | — | Path to the `docbrain-ingest` binary the server starts for an ingest triggered from the console or API. Unset uses the binary next to the server, then `docbrain-ingest` on the path. |
| `INGEST_CONCURRENCY` | `10` | Confluence pages fetched and processed at once during a Confluence ingest. A value that is not a number stops the ingest. |
| `DOCBRAIN_SELF_INGEST` | `true` | Index DocBrain's own documentation, from `DOCBRAIN_DOCS_PATH`, so it can answer questions about itself. The shipped `config/default.yaml` sets `true`; the code default is `false` when the `ingest:` block is absent. Skipped when the directory does not exist. |
| `DOCBRAIN_DOCS_PATH` | `./docs` | Directory of documentation that self-ingest reads. |

### Connector scheduler

The scheduler and sync engine for HTTP connectors. Set in the `connectors:` block or by these variables.

| Variable | Default | Description |
|---|---|---|
| `CONNECTOR_ENABLED` | `true` | Run the connector scheduler. `false` stops all connector syncs. |
| `CONNECTOR_MAX_CONCURRENT_SYNCS` | `3` | Connector syncs that may run at once. |
| `CONNECTOR_MAX_PAGES_PER_SYNC` | `200` | Most pages one sync fetches. |
| `CONNECTOR_MAX_DOCS_PER_SYNC` | `5000` | Most documents one sync ingests. |
| `CONNECTOR_MAX_DOCUMENTS_PER_SYNC` | `5000` | The same limit as `CONNECTOR_MAX_DOCS_PER_SYNC`, read from the config file. Once one sync has listed this many documents it stops listing and the rest wait for the next sync. When both are set, `CONNECTOR_MAX_DOCS_PER_SYNC` wins. |
| `CONNECTOR_FETCH_BATCH_SIZE` | `50` | Documents asked for in one fetch call to a connector. |
| `CONNECTOR_REQUEST_TIMEOUT_SECS` | `30` | Seconds before one HTTP call to a connector times out. |
| `CONNECTOR_SYNC_TIMEOUT_SECS` | `3600` | Seconds one whole sync may run (the default is one hour). |
| `CONNECTOR_MAX_RESPONSE_BYTES` | `10485760` | Largest response body accepted from a connector, in bytes (10 MB). |
| `CONNECTOR_CIRCUIT_BREAKER_THRESHOLD` | `5` | Failures in a row after which a connector is switched off. |
| `CONNECTOR_ALLOW_INTERNAL` | `false` | Let connectors call private or internal addresses. Leave it off unless a connector genuinely lives on your network, because it removes the guard against server-side request forgery. |

## Chunking

Controls how documents are split into chunks for embedding. See **[docs/chunking.md](chunking.md)** for tuning guidance and re-ingest instructions.

| Variable | Default | Description |
|---|---|---|
| `CHUNK_SIZE` | `1500` | Target chunk size in characters |
| `CHUNK_OVERLAP` | `200` | Overlap between adjacent paragraph-split chunks |

## RAG Pipeline

| Variable | Default | Description |
|---|---|---|
| `RAG_CLAIM_VERIFICATION` | `true` | Before returning an answer, check any file paths it cites against the file listings recorded when your git sources were ingested. A path that has moved is corrected; one that has been deleted is flagged, dated to the commit the listing came from. A path is flagged only when a connected repository holds its first two folders (`content/en/docs/…`), so files of a codebase you have not connected are no longer reported missing just because they share a top-level folder such as `docs/` or `scripts/`. When the top folder's name starts with `.` (a tool's folder such as `.github/`), the layout below it is the tool's and the same in every repository, so a connected repository must hold the path's first three folders instead (`.github/actions/setup/…`). The cost: a file directly under such a two-level folder (`.github/workflows/deploy.yml`) that is deleted from a connected repository is no longer flagged in answers. One exception remains: a file of an unconnected codebase under an ordinary two-level folder that a connected repository also holds (for example `tests/integration/`) can still be reported missing. A path found in any connected repository always passes. Deterministic — no extra LLM call — and inert until a git source has recorded a listing. |
| `PREMISE_MONITOR_ENABLED` | `true` | Enable a standing monitor that re-verifies file-path premises in captured knowledge fragments against your connected sources' file listings every `PREMISE_SWEEP_INTERVAL_SECS` seconds (default 300). Fragment-index events trigger extraction and initial verification of the new fragment's premises immediately, rather than waiting for the next sweep; re-verification of existing premises happens on the next sweep. The monitor is fully deterministic — no LLM calls, no extra network requests in the check path. A premise verified at capture that later disappears fires a `premise.broken` event (webhook-subscribable, event-log persisted); restoration fires `premise.restored`. Premises never verified at capture remain `dormant` and never alert. Note: `uncheckable` means "no connected source can currently speak to this premise", not "broken" — it never triggers an alert. |
| `PREMISE_SWEEP_INTERVAL_SECS` | `300` | Seconds between premise monitor sweeps. Each sweep reads the premises a connected source could settle, in pages of 1,000, and settles each one, so this is the only dial on that cost: raise it on a large premise table to spend less, lower it to notice a broken claim sooner. Values below `5` are raised to `5` — a zero period is rejected by the timer, and a one-second sweep spends a core re-deriving verdicts that cannot have changed. Fragment-index events still verify a new fragment's premises immediately, independent of this interval. Helm: `premises.sweepIntervalSeconds`. |
| `PREMISE_DOCUMENT_EXTRACTION` | `false` | Also extract premises from ingested documents, not only from captured fragments. Off by default, and deliberately so: turning it on runs a one-time backfill over the whole corpus and creates a premise row for every path-shaped claim in it (measured over 635 real documentation files at roughly 6.5 claims per document, so a 39,000-document instance writes on the order of a quarter of a million rows on its first pass). The sweep skips the premises nothing can settle. Deterministic — no LLM call. `docs/premises.md` describes the document pass and its cost. Helm: `premises.documentExtraction`. |
| `RAG_CACHE_TTL_HOURS` | `24` | How long a cached answer is served (hours). `0` turns the answer cache off. A cached answer is served only to the exact same question, character for character, asked outside a conversation — a reworded question, or one about a different file or service, runs the full pipeline. (`RAG_CACHE_THRESHOLD`, the old similarity threshold, is retired and ignored; the server logs a warning when it is set as an environment variable or Helm value `rag.cacheThreshold`; as `rag.cache_threshold` in a config file it is ignored without a warning.) |
| `FAST_LLM_PROVIDER` | — | Run the FAST model on a **different provider** than `LLM_PROVIDER`. Unset — or naming the same provider — keeps the previous behaviour, where the fast model is another model on the primary provider. This exists because the two models do different jobs: side-calls (query rewriting, the retrieval judge, the MCP meta-gateway's argument builder) need **speed**, and one of them runs *inside* tool dispatch where the whole request waits on it, while synthesis needs depth. On a self-hosted single GPU those requirements conflict — measured 2026-09-06/07, an argument-builder call needing ~8s of GPU waited over 30s behind synthesis on Ollama (which serialises per model) and timed out the dispatch; giving it a second local model instead traded that queueing for weight eviction. `FAST_LLM_PROVIDER=anthropic` with a local `LLM_PROVIDER=ollama` resolves both without giving up self-hosted synthesis. Providers needing async initialisation (`bedrock`, `vertex_ai`) are refused here with a message naming the variable, rather than failing later as an unexplained side-call error. Supply that provider's own credentials as usual (e.g. `ANTHROPIC_API_KEY`). |
| `RAG_RETRIEVAL_LADDER_ENABLED` | `false` | Synthesize **two** answers — one from the indexed corpus alone, one that also includes live MCP tool output — and let an LLM judge keep the better one. Off by default. It exists because the normal fused path puts corpus chunks *and* live tool output into one prompt, where live output can make an otherwise good corpus answer **worse**: measured 2026-09-04, a FluxCD question answered at 0.90 confidence from the corpus alone fell to 0.15 once a Confluence tool returned results into the same prompt. With the ladder on, live data can only help. The extra cost is paid **only when a tool fires** — a corpus-only question runs a single synthesis exactly as before — and the two syntheses run concurrently, so the added wall-clock is roughly one judge call rather than a whole second answer. |
| `RAG_RETRIEVAL_LADDER_JUDGE_TIMEOUT_MS` | `10000` | Hard timeout for the judge call. On timeout the answer with the higher self-graded confidence wins — a safe degradation, not a failure. This was `1500`, which no local model can meet: a judge call against a 14B model measured 46-120s, so self-hosted deployments silently never ran the judge at all and always took the fallback. Raise it to get a real judgement from a local model; lower it to bound added latency and accept the fallback. |
| `RAG_RETRIEVAL_LADDER_GRAPH_THRESHOLD` | `0.5` | Below this winning confidence, the answer is augmented with graph-derived related-topics text. |
| `RAG_RETRIEVAL_LADDER_JUDGE_MODEL_ID` | — | Model for the judge call. Unset uses the configured fast model. |
| `RAG_CONTEXT_WINDOW_BUDGET_BYTES` | `65536` | Total byte budget for expanded **document** context across all documents; overflow is trimmed from the lowest-ranked end. The real limit is your model's *token* window (~3.2 bytes/token measured), so lower it if your model's window is small — and watch the `stage="rag.staged.context_window"` log line, because providers truncate an over-long prompt silently rather than erroring. |
| `RAG_CONTEXT_WINDOW_CHUNKS` | `3` | Neighbouring chunks fetched on each side of a matched chunk, so the answer sees the passage and not only the fragment. `0` turns the expansion off. Lookup questions (find, who owns) never expand. |
| `RAG_LIVE_DATA_BUDGET_BYTES` | `32768` | **Aggregate** ceiling on the LIVE DATA block — the concatenation of *every* MCP tool block spliced into one synthesis prompt. Each tool's own output is already capped by `DOCBRAIN_MCP_DEFAULT_TOOL_OUTPUT_CAP_BYTES` (default 32768), so this setting is what bounds **several tools answering at once**; nothing else does. Measured 2026-09-04: a Confluence tool returned 103,479 bytes and reached the prompt as 8,475 after per-tool capping and redaction, so one tool rarely approaches this ceiling. Over budget the block is truncated with an explicit marker and a `warn` log (`live tool output exceeds the synthesis budget`), because a provider does not reject an over-long prompt — it drops the overflow silently. |
| `RAG_TOP_K` | `10` | Chunks retrieved per query. Higher = more context passed to LLM, higher token cost. |
| `RAG_MAX_PER_SOURCE` | `3` | Max chunks per **source group** in the final top-k, where the group is the chunk's `space`: the space key for Confluence (`ENG`, `OPS`, …) and the literal `local` for local-file ingest — *not* the `source_type`. A distribution cap: it stops one group crowding out the others. **Skipped entirely when fewer than two groups are eligible** — with a single group (local-only, one-space Confluence, …) it would otherwise cap total LLM context at 3 chunks regardless of `RAG_TOP_K`, which starves synthesis and presents as a weak model rather than a truncated context. With two or more groups it binds normally, and it may then leave the result under `RAG_TOP_K` by design: two groups at the default of 3 yield at most 6 chunks even with `RAG_TOP_K=10`. Raise this value if you want a small number of groups to fill top-k. |
| `RAG_MAX_PER_DOCUMENT` | `2` | Max chunks from any one **document** in the final top-k. Bounds redundancy: consecutive chunks of one document are frequently near-duplicates. Never relaxed — unlike the per-source cap, this is not about distribution across sources. |
| `RAG_BM25_BOOST` | `1.0` | BM25 keyword search weight relative to vector search. Raise (e.g. `2.0`) for corpora with lots of exact-match queries (error codes, tool names). |
| `SEARCH_MIN_SCORE` | `0.0` | Drop retrieved chunks below this relevance score. `0.0` keeps everything; `0.3`–`0.4` filters low-signal noise. |
| `OPENSEARCH_INDEX` | `docbrain-chunks` | Name of the document chunk index in OpenSearch |
| `OPENSEARCH_EPISODE_INDEX` | `docbrain-episodes` | Name of the episode (Q&A history) index in OpenSearch |
| `RAG_FRESHNESS_PRE_DIVERSITY` | — | Removed. It had re-enabled an old freshness multiplier on retrieval; the server ignores it and logs a warning when it is set. Delete it from your configuration. |
| `RAG_CANDIDATE_POOL_SIZE` | `200` | Candidates the search stage hands to the reranker for each question. More raises recall and reranker cost; fewer than about 100 hurts recall on large corpora. |
| `RAG_RRF_K` | `60` | Damping constant when keyword and vector results are merged by reciprocal rank. A larger value favours chunks that rank well in both lists; a smaller one favours each list's top hits. |
| `RAG_MIN_RELEVANCE_SCORE` | `0.40` | Retrieval floor: a chunk the reranker scores below this never reaches the model. See Grounding floors below for what lowering costs. |
| `RAG_DISPLAY_FLOOR` | `0.50` | Display floor: a chunk scored below this is never shown as a citation. |
| `RAG_CONFIDENCE_GATE` | `0.40` | Below this answer confidence the answer is returned as "not found" with no sources; the prompt forbids answering from general knowledge. |
| `RAG_AGENTIC_LOOP_ENABLED` | `false` | Turns on the agentic loop, which lets the model call connected tools over several rounds before it answers. `1`, `true`, `yes` or `on` enables it; any other value leaves it off (an unrecognised one logs a warning). |
| `RAG_AGENTIC_LOOP_BUDGET_MS_WEB` | `48000` | Wall-clock deadline of the loop, in milliseconds, for every surface except Slack. A whole number from `1000` to `120000`; anything else logs a warning and uses the default. `ASK_WALL_CLOCK_SECS` can shorten it further. |
| `RAG_AGENTIC_LOOP_BUDGET_MS_SLACK` | `60000` | As `RAG_AGENTIC_LOOP_BUDGET_MS_WEB`, for the Slack surface, which posts its answer when done and can wait longer. |
| `RAG_AGENTIC_LOOP_MAX_ROUNDS_WEB` | follows the deadline | Hard cap on tool rounds for every surface except Slack, from `1` to `10`. Unset, the cap is what the deadline pays for at 12 seconds a round (`4` at the default deadline). Set it only to cap cost whatever the time; to allow more steps, lengthen the deadline. An out-of-range value logs a warning and uses the derived cap. |
| `RAG_AGENTIC_LOOP_MAX_ROUNDS_SLACK` | follows the deadline | As `RAG_AGENTIC_LOOP_MAX_ROUNDS_WEB`, for Slack (`5` at the default deadline). |
| `RAG_AGENTIC_LOOP_CONFIDENCE_THRESHOLD` | `0.7` | The loop keeps going while its best answer confidence is below this, from `0.0` to `1.0`. Unset, it reads `RAG_CONFIDENCE_RETRY_THRESHOLD` first and uses `0.7` only if both are unset. A value that is set but invalid uses `0.7` without trying the other. |
| `RAG_CONFIDENCE_RETRY_THRESHOLD` | `0.7` | The older name of the loop's confidence threshold, still read when `RAG_AGENTIC_LOOP_CONFIDENCE_THRESHOLD` is unset, so a cluster that tuned it keeps its value. Same range and rules. Prefer the new name. |
| `RAG_MAX_CHUNKS_PER_DOC` | `2` | Most chunks of one document a single retriever may contribute before merging, so one long page cannot fill a retriever's list. Each retriever is counted separately. |
| `RAG_FRESHNESS_WINDOW_DAYS` | `7` | Documents updated within this many days are always added to the candidate pool, so fresh captures get a chance to be reranked. `0` turns that off. |
| `RAG_ENTITY_CACHE_TTL_SECS` | `300` | Seconds the entity-to-space lookup is cached before it is refreshed. |
| `RAG_MAX_REWRITES` | `2` | Most alternative phrasings of a question that are searched alongside it. Each costs one more keyword and one more vector search. `0` turns rewriting off. |
| `RAG_COMPOUND_DECOMPOSE` | `true` | Split a question that asks several things into sub-questions so each is reranked on its own. `false` or `0` skips the extra fast-model call. |
| `RAG_RERANK_TITLE_ENRICH` | `true` | Give the reranker each chunk's title, heading and space along with its text. `false` or `0` sends the text alone. |
| `RAG_TRACE_DETAIL` | `false` | `true` or `1` logs one line per final result for each question. Verbose; for diagnosing a missing source. |
| `RAG_FUSION_PREFER_FUSED` | `false` | `true` always answers from one pass over the indexed documents and every live-tool result together, and never uses the winner-pick ladder. |
| `RAG_CACHE_THRESHOLD` | — | Retired. The answer cache now serves only an exact repeat of a question, so a similarity threshold means nothing. The server ignores it and logs a warning when it is set. To turn the cache off, set `RAG_CACHE_TTL_HOURS=0`. |

### Tool-output caps (the truncation chain)

Three coordinated caps bound how much raw tool output reaches the synthesis prompt. They form a chain — **all three must rise together**, because the smallest in the chain truncates regardless of the others. The defaults let a high-volume search (which can return 43-63KB of raw results, e.g. Slack search) keep ~32KB instead of being truncated to a fraction. The two `MCP` caps are read directly from the env var by the manifest validator (which lives in a crate that does not depend on the main config), so override the env var — not just the YAML — when tuning them. Unset or invalid (non-numeric / ≤ 0) values fall back to the default.

| Variable | Default | Description |
|---|---|---|
| `DOCBRAIN_MCP_OUTPUT_CEILING_BYTES` | `32768` | Hard upper bound the manifest validator enforces on any per-tool `output_size_cap_bytes`. A per-tool cap above this is rejected at manifest load. |
| `DOCBRAIN_MCP_DEFAULT_TOOL_OUTPUT_CAP_BYTES` | `32768` | Output cap inherited by tools discovered dynamically that ship no per-tool cap (e.g. Slack search). Must be ≤ the ceiling above. |
| `DOCBRAIN_EVIDENCE_BUFFER_CAP_BYTES` | `65536` | Shared evidence-text budget across **all** tools in one agentic-loop round — the real bottleneck. Sized at 2× the per-tool cap so a maxed single-source result still leaves headroom for other sources. |
| `DOCBRAIN_MCP_JQL_RECENCY_BOUND_DAYS` | `180` | Recency window (in days) the gateway appends to an **unbounded full-text** Jira search. When a Jira search's JQL uses the `text ~` full-text operator with no time window and no `project=`/`key=` clause, the gateway adds `AND updated >= -<N>d` so Jira hits the date index instead of scanning the whole instance (which times out at the 10s tool budget). Already-bounded queries are left untouched. |
| `DOCBRAIN_SUFFICIENCY_CRITIC_ENABLED` | `true` | Master kill-switch for the **sufficiency critic** — a scope-relative answer-quality gate that runs after synthesis. It judges whether the answer covers the question's own sub-questions, grounded in the retrieved sources; if not, it drives one bounded recovery round on the named gap, and if still short it returns an honest, specific gap instead of a confident-but-incomplete answer. **ON by default** (the critic is part of the product). Disable only with `false`/`0`/`no`/`off` — an operator turns it off without a code rollback if it misbehaves in a deployment. Fail-OPEN: any critic error ships the answer unchanged. |
| `DOCBRAIN_SUFFICIENCY_CRITIC_MAX_RECOVERY_ROUNDS` | `1` | Hard cap on the number of recovery rounds the critic may force (range `0..=2`). Worst case adds N extra rounds, always inside the agentic loop's existing round/wall-clock cliffs. `0` disables recovery (the critic then only appends honest gaps, never re-queries). |
| `DOCBRAIN_SUFFICIENCY_CRITIC_SKIP_ABOVE_CONFIDENCE` | `0.85` | Skip the critic entirely when the mechanical confidence is already ≥ this value (a confidently-answered question doesn't need re-judging). Range `0.0..=1.0`. |
| `DOCBRAIN_SUFFICIENCY_CRITIC_MIN_COVERAGE` | `0.9` | The coverage-fraction dial the critic prompt judges against (the "how complete is complete" threshold on the question's own sub-questions). Range `0.0..=1.0`. |
| `DOCBRAIN_SUFFICIENCY_CRITIC_MIN_RECOVERY_BUDGET_MS` | `20000` | Minimum wall-clock headroom (ms) required to **allow** a critic-forced recovery round. A recovery round costs roughly one tool dispatch plus the final synthesis; when fewer than this many ms remain in the surface's wall-clock budget, the critic ships the candidate as an **honest-gap** answer instead of forcing a round the budget cliff would kill (which previously caused a request-level timeout). Set `0` to disable the guard (always allow recovery within the round cap). |
| `ASK_WALL_CLOCK_SECS` | `600` | Hard cap on a whole `/ask`, in seconds. Every stage inside the request already has a budget — the agentic loop's wall clock, the critic's recovery floor, each tool call's timeout — and none of them bounds their **sum**. A measured question ran **1077 seconds** (eighteen minutes) with every stage inside its own limit; the retrieval pipeline accounted for 25 of those seconds and the rest was the loop and synthesis making model calls. Nobody waits that long, and on a shared instance the request holds a connection and its share of the pools throughout. The default is deliberately generous (the slowest legitimate answer measured in the same run was 248s), so this bounds the pathological case without truncating a slow honest one. **The cap is enforced inside the pipeline**, at the two ask entry points, so every surface is bound by it — SSE, JSON, Slack, the webhooks, eval — rather than only the JSON handler that used to own the timer. It also **clamps the agentic loop's own wall clock** (`RAG_AGENTIC_LOOP_BUDGET_MS_*`) down to whatever is left of the turn, which in turn bounds every tool dispatch inside the loop. **A stopped turn is not an error.** It returns the answer it had — the loop's text when there is one, which a streaming asker has already been shown — labelled with a `deadline` degradation naming the limit and the stage it died in; a one-sentence body saying the question was stopped is returned only when nothing was produced at all. Either way the turn is recorded: the `episodes` row carries `metadata.stopped.reason = "deadline"` alongside `elapsed_ms`, `budget_ms` and the stage, so the question that broke the instance is findable. (One exception, shared with every other ending: under ACL enforce mode a turn that retrieved nothing writes no episode at all, because the row would pair the question with an empty document list and tell a later reader what the asker was denied.) A row carrying `metadata.stopped` is **not** counted as an unanswered question by any reader that turns unanswered questions into documentation debt — the gap analyzer, the Knowledge Gap Map, the weekly digest, the retrieved-unhelpful detector, the consolidation job and the enterprise health report's `coverage_gaps` all exclude it — because a turn that never reached an answer is evidence about the instance, not about the corpus. Both arms of `/api/v1/ask` also carry a transport **backstop** at the cap **plus a five-second grace** — the JSON arm's request timeout and the SSE forwarder's — for a pipeline whose own clock failed to land. Reaching one is a bug report, not a slow question, and the two arms do not leave the same trace. The JSON arm answers **504** and records **nothing** — the timeout fires outside the pipeline, and the pipeline that should have written the turn is the thing that did not stop. The SSE arm sends an `error` then `done` and **does record the turn**: the forwarder still holds the question, so it writes the episode itself with `metadata.stopped.reason = "deadline_backstop"` (the same key and shape, under a different reason, as a turn the pipeline's own clock cut — and the reason a closed tab writes is `client_disconnect`). **One limit on how exact the cap is:** it is per **ask**, not per request, so an N-section doc generation is bounded at N × this value. (The agentic loop's last model call — the one that writes the answer text — used to be the other limit, bounded by `LLM_TIMEOUT_SECS` rather than by this cap, so a model hanging there ended the turn at cap + up to two minutes and the transport backstop above reached the asker first. That call now runs under the turn's clock, and a hang there ends the turn at the cap with the one-sentence stopped body.) Set `0` to disable the cap. |
| `DOCBRAIN_FRESHNESS_CRITIC_ENABLED` | `true` | Master kill-switch for the **freshness critic** — the doc-generation reviewer that runs after a draft is written. It segments the draft (and, for augments, the existing doc) into atomic claims, retrieves ACL-filtered evidence per claim, and **flags only the claims the evidence CONTRADICTS** (conservative-KEEP: uncertainty never flags). **ON by default**. Disable only with `false`/`0`/`no`/`off`. Fail-OPEN: any critic error ships the draft unchanged. |
| `DOCBRAIN_FRESHNESS_CRITIC_MAX_CLAIMS` | `60` | Hard cap on the number of claims validated per draft (across new + existing-doc content). Over-cap claims are reported as **unchecked** in the freshness report, never silently dropped. Bounds the worst-case LLM cost. Range `1..=1000`. |
| `DOCBRAIN_FRESHNESS_CRITIC_BATCH_SIZE` | `12` | How many claims (with their evidence) go into one batched judgment LLM call. The number of judgment calls is `ceil(claims / batch_size)`. Range `1..=100`. |
| `DOCBRAIN_FRESHNESS_CRITIC_EVIDENCE_PER_CLAIM` | `5` | How many evidence snippets to retrieve per claim. Caps the per-claim retrieval fan-out and the judgment-prompt size. Range `1..=50`. |
| `DOCBRAIN_FRESHNESS_CRITIC_MIN_BUDGET_MS` | `20000` | Minimum wall-clock headroom (ms) required to **run** the critic at all. When fewer than this many ms remain in the generation budget, the critic is skipped (fail-open: the draft ships without a freshness report rather than risk a budget-cliff stall). Set `0` to disable the guard (always run). |
| `DOCBRAIN_SUPPORT_CRITIC_ENABLED` | `true` | Master kill-switch for the **support critic** — the doc-generation GROUNDING reviewer. It extracts the draft's claims and flags the ones **NO supplied source supports** (fabrication), plus `NEEDS INPUT:` markers the sources actually cover (the inverted self-flag). Distinct from the freshness critic (which flags only CONTRADICTED claims): the support critic flags UNSUPPORTED ones. **ADVISORY** — it never blocks generation; the findings are surfaced in the grounding report for the reviewer. **ON by default**. Disable only with `false`/`0`/`no`/`off`. Fail-OPEN: any critic error ships the draft unchanged. |
| `DOCBRAIN_SUPPORT_CRITIC_MAX_CLAIMS` | `40` | Hard cap on the number of claims grounding-checked per draft. Bounds the worst-case LLM cost of the support critic. Range `1..=200`. |
| `DOCBRAIN_SUPPORT_CRITIC_NOFAB_ENFORCE` | `true` | **NoFab enforcement** — turns the support critic's advisory findings into a structural guarantee. When **ON** (default), any **freshly-generated** section the critic found ungrounded is **degraded in place to a `NEEDS INPUT` gap** (section-granular), so a fabricated claim can never reach the reader as authoritative fact — it becomes an honest, labelled gap instead. In a `--target` update, only fresh additions are enforced; the existing document's content is preserved byte-exact and its unverified claims are surfaced as a separate review-only list (`preexisting_unverified`), never rewritten. Safe by construction (degrading never invents). Disable only with `false`/`0`/`no`/`off`. |
| `DOCBRAIN_REGEN_LOOP_ENABLED` | `true` | Master kill-switch for the **regenerate loop** — the reviewer-loop step that consumes the freshness critic's flags and **regenerates a draft until they clear**. The UI is human-in-loop (each click is one round, capped at `MAX_REVISIONS`); the CLI/`generate` path auto-reviews (capped at `MAX_ROUNDS`). Feedback shapes the **writing prompt only** — it can **never** clear a contradicted flag (the critic re-derives flags from evidence). The round cap is the hard termination backstop: the loop never runs forever and never silently ships unresolved flags. Disable only with `false`/`0`/`no`/`off`. |
| `DOCBRAIN_REGEN_LOOP_MAX_ROUNDS` | `3` | Hard cap on **CLI/`generate` auto-review** rounds. After the first generation, if claims are still flagged, the loop regenerates up to this many extra rounds, stopping early on a plateau (a round that resolves no previously-flagged claim). Range `1..=20`. |
| `DOCBRAIN_REGEN_LOOP_MAX_REVISIONS` | `10` | Cap on **UI human-in-loop** revisions per draft. Each "Regenerate" click writes a new revision linked to its parent; this bounds the chain depth so a reviewer can't loop indefinitely. Range `1..=100`. |
| `DOCBRAIN_REGEN_LOOP_MIN_BUDGET_MS` | `20000` | Minimum wall-clock headroom (ms) required to **start** another round. When less remains, the loop exits honestly (ships the latest draft + its unresolved flags) rather than starting a round the budget cliff would kill mid-flight. Set `0` to disable the guard. |
| `DOCBRAIN_GENERATE_HOLLOW_RATIO_PCT` | `50` | The density threshold for the hollow-doc guard (DELTA #9). The guard refuses with **`422`** ONLY when a draft has **no inline seed, no High-tier authoritative grounding, and no Low-tier live grounding either**, AND at least this percent of its `##` sections are unanswered `NEEDS INPUT:` placeholders. A no-seed/no-High draft that **does** have live/Low grounding ships as **unverified** (flagged + barred from auto-publish) rather than refusing. The guard applies to **short docs too** (the old `…_MIN_SECTIONS` floor was removed; the only floor left is `section_count > 0`). `50` = "at least half the document is placeholders". Tunable `0`–`100`. |
| `DOCBRAIN_RECONCILE_ENABLED` | `true` | Master kill-switch for **in-place reconcile**. When the reviewer flags a section of an existing page as stale, reconcile replaces **only that section in place** (in storage byte-space) instead of appending a fresh block — so the page never accumulates duplicate-but-newer content. Defaults **ON**: the publish-time version check, the lint floor, and fail-open-to-append make it survivable live. Disable only with `false`/`0`/`no`/`off`. |
| `DOCBRAIN_RECONCILE_MAX_SECTIONS` | `5` | Max sections reconciled in a single draft, bounding the per-draft LLM + splice cost. Above this, remaining flagged sections route to a human rather than being reconciled in the same pass. Range `1..=50`. |
| `DOCBRAIN_MERGE_ENABLED` | `true` | Master kill-switch for the **merged-doc update**. When you generate against an existing target document, the output is the **full merged document** — unchanged sections preserved byte-exact, changed sections rewritten, and new sections surfaced — plus a structured per-section change manifest, so the result can replace the whole doc with confidence about what changed. Defaults **ON**: unchanged sections are byte-exact by construction, an edit that can't splice safely bails (the section is left unchanged), and nothing is ever published. Disable only with `false`/`0`/`no`/`off`. |
| `DOCBRAIN_MERGE_MAX_SECTIONS` | `60` | Max existing sections fed to the merge decision in one pass, bounding the prompt cost. Sections beyond the cap are kept verbatim (never proposed for change), so the cap limits cost, not correctness. Range `1..=500`. |
| `DOCBRAIN_MERGE_MAX_TOKENS` | `4096` | Max output tokens for the merge decision call (the model returns only the changed spans, not the whole document). Range `512..=32768`. |
| `DOCBRAIN_EMPTY_RESULT_FLOOR_BYTES` | `256` | Byte floor below which a **search/list** tool's `ok` result is treated as *near-empty* (a 200-but-no-results response, e.g. a "no matches" JSON envelope). When a search returns fewer bytes than this, the agentic loop forces **one** broaden-retry of the same tool with a wider query (drop qualifiers/filters), bounded once per tool. Read tools and failed tools are never broadened. |

## Reranker (Stage 3 of Retrieval)

The reranker rescores the candidate pool from stage 2 with a cross-encoder, producing calibrated `[0, 1]` relevance scores that drive the downstream grounding floors. DocBrain supports every major hosted rerank API through a single HTTP client parameterised by a "dialect", so adding a new provider is typically a config change, not a code change.

**Built-in providers:** `bedrock`, `cohere`, `voyage`, `jina`, `mixedbread`, `pinecone`, `ollama`. Plus `custom` for any other Cohere-family API.

| Variable | Default | Description |
|---|---|---|
| `RAG_RERANK_PROVIDER` | `none` | `none` \| `bedrock` \| `cohere` \| `voyage` \| `jina` \| `mixedbread` \| `pinecone` \| `ollama` \| `custom` |
| `RAG_RERANK_MODEL_ID` | *(provider default)* | Override the default model id (e.g. `rerank-v3.5`, `rerank-2`, `jina-reranker-v2-base-multilingual`) |
| `RAG_RERANK_TOP_N` | `200` | Max candidates handed to the reranker per query. Larger = better recall, linearly higher cost. |
| `RAG_RERANK_BATCH_SIZE` | `100` | Split large pools into batches of this size. Clamped to `[1, 1000]`. |
| `RAG_RERANK_TIMEOUT_SECS` | `10` | Per-request timeout. Reranker is on the critical path of every `/api/v1/ask` request. |

### Hosted provider credentials

| Provider | API key env var | Notes |
|---|---|---|
| `bedrock` | *(IAM via default AWS credential chain)* | Requires `--features bedrock` at build time |
| `cohere` | `COHERE_RERANK_API_KEY` | [console.cohere.com](https://dashboard.cohere.com/api-keys) |
| `voyage` | `VOYAGE_API_KEY` | [voyageai.com](https://dash.voyageai.com/) |
| `jina` | `JINA_API_KEY` | [jina.ai](https://jina.ai/api-dashboard/) |
| `mixedbread` | `MIXEDBREAD_API_KEY` | [mixedbread.ai](https://www.mixedbread.ai/dashboard) |
| `pinecone` | `PINECONE_API_KEY` | [pinecone.io](https://app.pinecone.io/) — uses `Api-Key` header, not Bearer |
| `ollama` | *(none — local)* | See `RAG_RERANK_OLLAMA_BASE_URL` below |

### Ollama (local, no key)

Ollama has no first-class rerank endpoint. DocBrain approximates rerank by computing cosine similarity between query and document embeddings from any Ollama embedding model. This is a **bi-encoder**, not a true cross-encoder — quality is lower than hosted providers, but it's fully local and air-gapped.

| Variable | Default | Description |
|---|---|---|
| `RAG_RERANK_OLLAMA_BASE_URL` | `http://localhost:11434` | Ollama server URL |
| `RAG_RERANK_MODEL_ID` | `nomic-embed-text` | Any Ollama embedding model |

For true cross-encoder quality locally, run `bge-reranker` or `mxbai-rerank` behind a small HTTP server and use `provider: custom` (below).

### Custom provider — plug-and-play for any Cohere-family rerank API

Set `RAG_RERANK_PROVIDER=custom` and fill the fields below to wire a new rerank provider without rebuilding DocBrain. Defaults match Cohere's request/response shape; override any field whose JSON key differs.

| Variable | Required | Default | Description |
|---|---|---|---|
| `RAG_RERANK_CUSTOM_BASE_URL` | ✅ | — | Full POST URL, e.g. `https://api.example.test/v1/rerank` |
| `RAG_RERANK_CUSTOM_API_KEY_ENV` | ✅ | — | Name of another env var that holds the API key (the key is never persisted in config.yaml) |
| `RAG_RERANK_MODEL_ID` | ✅ | — | Model id to send in the request body |
| `RAG_RERANK_CUSTOM_AUTH_STYLE` |  | `bearer_token` | `bearer_token` or `custom_header` |
| `RAG_RERANK_CUSTOM_AUTH_HEADER_NAME` | only with `custom_header` | — | Header name, e.g. `Api-Key` |
| `RAG_RERANK_CUSTOM_DOCUMENTS_FIELD` |  | `documents` | Request JSON key for the documents array |
| `RAG_RERANK_CUSTOM_TOP_N_FIELD` |  | `top_n` | Request JSON key for the top-N limit |
| `RAG_RERANK_CUSTOM_RESULTS_FIELD` |  | `results` | Response JSON key for the results array |
| `RAG_RERANK_CUSTOM_SCORE_FIELD` |  | `relevance_score` | Response JSON key for the score |

See [rerank-providers.md](rerank-providers.md) for complete per-provider examples and the "how to add a new provider in 2 minutes" recipe.

### Grounding floors — what lowering actually costs

The reranker feeds four downstream floors in `rag.*` that are the single biggest quality lever in DocBrain. Recommended defaults — calibrated for a cross-encoder reranker like Cohere Rerank v3.5 or equivalent:

| Floor | Default | Meaning | What lowering costs |
|---|---|---|---|
| `rag.min_relevance_score` | `0.40` | Chunks below this never reach the LLM | **Hallucination risk** — the LLM sees weaker evidence and writes confident answers from chunks that only tangentially match |
| `rag.display_floor` | `0.50` | Chunks below this are never shown as citations | **User trust** — tangentially-related docs appear in the sources list and erode credibility |
| `rag.confidence_gate` | `0.40` | Below this, the answer is returned as "not found" with no sources | Sources render on low-confidence answers that may mislead users |

**Calibration:** a cross-encoder's `[0, 1]` score is **not** a percentage. For Cohere Rerank v3.5 and similar models, `> 0.70` means "directly answers the question", `0.50–0.70` is "strong supporting evidence", `0.40–0.50` is "topically related", `0.30–0.40` is "shares keywords but usually noise". Defaults draw the lines at "topically related" for retrieval and "strong evidence" for citation display.

**When `rerank.provider = "none"`** these floors gate on raw BM25/vector scores which are NOT calibrated — set all four to `0.0` and bound results with `top_k`.

See [rerank-providers.md § "Tuning the grounding floors"](rerank-providers.md#tuning-the-grounding-floors) for the full explanation, the calibration bands, and the citation-debugging recipe.

### Source suppression (feedback learning loop)

When a user marks a **specific source** within an answer as not-relevant (the per-source thumbs-down on web, CLI, and Slack), DocBrain records the event and — once enough independent feedback accumulates — **down-ranks that document in retrieval for similar future questions**. "Similar" is resolved through the episodes already recalled on the retrieval hot path, so there is no extra embedding or clustering cost. The penalty is applied to the document's retrieval score, never a hard drop, so a flagged document that is the only available evidence still surfaces rather than producing an empty answer.

A document is suppressed when it crosses **either** threshold gate — enough total events **or** enough distinct users — so a single click can never unilaterally bury a document. Set both `min_*` to `0` to disable suppression entirely.

| Setting | Env var | Default | Meaning |
|---|---|---|---|
| `rag.suppression.min_feedback_count` | `RAG_SUPPRESSION_MIN_FEEDBACK_COUNT` | `2` | Total not-relevant events on a document (across recalled episodes) before it is suppressed. `0` disables this gate. |
| `rag.suppression.min_unique_users` | `RAG_SUPPRESSION_MIN_UNIQUE_USERS` | `2` | Distinct users who flagged the document before it is suppressed. Anonymous (no user) events count toward the event total but never toward the distinct-user quorum. `0` disables this gate. |
| `rag.suppression.rag_penalty_factor` | `RAG_SUPPRESSION_RAG_PENALTY_FACTOR` | `0.1` | Multiplier applied to a suppressed document's retrieval score. Range `(0, 1]`: `1.0` = no penalty, smaller = stronger down-rank. |

## Autopilot / Gap Analysis

The Autopilot module automatically identifies documentation gaps from user query patterns and closes the feedback loop by generating and publishing documentation back to your knowledge sources.

**Autopilot is enabled by default.** Teams can opt out by setting `AUTOPILOT_ENABLED=false`.

| Variable | Default | Description |
|---|---|---|
| `AUTOPILOT_ENABLED` | `true` | Set to `false` to disable autopilot features |
| `AUTOPILOT_LOOKBACK_DAYS` | `30` | Days of query history to analyze for gaps |
| `AUTOPILOT_CLUSTER_THRESHOLD` | `0.82` | Cosine similarity for clustering queries (0.65=loose, 0.85=strict) |
| `AUTOPILOT_MIN_CLUSTER_SIZE` | `3` | Minimum number of queries in a cluster to be considered a gap |
| `AUTOPILOT_MIN_UNIQUE_USERS` | `2` | Minimum distinct users that must hit the same gap |
| `AUTOPILOT_MIN_NEGATIVE_RATIO` | `0.15` | Minimum fraction of queries on a topic that must have negative feedback |
| `AUTOPILOT_MAX_CLUSTERS` | `50` | Maximum gap clusters to persist per analysis run |
| `AUTOPILOT_MAX_EPISODES` | `500` | Maximum negative episodes to load per analysis run |
| `AUTOPILOT_ANSWER_TOPIC_MAX_EMBED_FAILURES` | `3` | How many hourly topic runs may fail to embed one question before that question is no longer read, so one question the embedder always rejects cannot hold the step back. Used only when answer topics are on. |
| `AUTOPILOT_ANSWER_TOPICS_ENABLED` | `false` | Group answered questions into topics and show a capture's author which topics its answers covered. Off by default until topics pass their quality check: off, the hourly topic step does not run and no topic is shown anywhere. |
| `AUTOPILOT_GAP_ANALYSIS_INTERVAL_HOURS` | `6` | How often the background gap analysis scheduler runs |
| `AUTOPILOT_AUTO_DRAFT` | `false` | Automatically generate drafts for qualifying gaps (no human trigger). Set to `true` to enable. |
| `AUTOPILOT_AUTO_DRAFT_SEVERITY` | `critical` | Minimum gap severity for auto-drafting: `critical`, `high`, `medium`, or `low` |
| `AUTOPILOT_CRITICAL_USERS` | `5` | Unique users needed for breadth score to reach 1.0. Lower this for small teams. |
| `AUTOPILOT_CRITICAL_SIGNALS` | `15` | Negative signals needed for volume score to reach 1.0. Lower for low-traffic deployments. |
| `AUTOPILOT_CRITICAL_THRESHOLD` | `0.75` | Composite score cutoff for "critical" severity. |
| `AUTOPILOT_HIGH_THRESHOLD` | `0.55` | Composite score cutoff for "high" severity. |
| `AUTOPILOT_MEDIUM_THRESHOLD` | `0.35` | Composite score cutoff for "medium" severity. |
| `AUTOPILOT_TARGET_MIN_SCORE` | `45.0` | Corpus-probe relevance floor: minimum OpenSearch hybrid (BM25+kNN, unbounded) probe score a candidate target doc must reach before autopilot auto-picks it to augment a `poor_coverage` gap. Below this the cluster is marked "needs human pick". Distinct from `VERIFY_CORPUS_MIN_SCORE`. |
| `GENERATED_DOCS_RETENTION_DAYS` | `90` | Retention window (days) for persisted ad-hoc `generate` runs shown in the web `/generate` History view (`generated_documents` table). Rows older than this are purged by a daily job, and the History list/detail also filter to this window. Bounds the data-at-rest exposure of the persisted document body (which is owner-scoped). Set `0` to keep indefinitely. |
| `DOCBRAIN_GENERATE_SOURCE_STALENESS_WARN_DAYS` | `365` | Staleness warn window (days) for a generation's grounding sources. A grounding source whose DocBrain-controlled ingest date is older than this is flagged **stale** in the trust headline ("grounded in a 2023 page"); within it, **fresh**. A source with no known / unparseable date renders **unknown** — explicitly distinct from fresh, never assumed fresh. Drives the per-source freshness flag on a generated artifact. |
| `DOCBRAIN_GENERATE_MAX_SECTION_ASKS` | `8` | Max number of per-section retrievals a single `generate` run fires. The document is decomposed into sections, and the same retrieval engine `/ask` uses is driven once per section to gather grounded content — so a generated section is filled only when that retrieval returns **high-tier, on-topic** evidence; otherwise it becomes `NEEDS INPUT`. Bounds the per-document retrieval cost (each section retrieval runs its own budgeted loop); sections beyond the cap are reported as not-asked, never silently dropped. Clamped to `0..=24`. Set `0` to disable per-section retrieval (falls back to a single corpus gather). |
| `DOCBRAIN_GENERATE_VERIFY_SECTION_QUERY_LLM` | `true` | Enable the per-section retrieval-**query** LLM rewrite. When `true`, one batch LLM call rewrites each outline heading into an `/ask`-quality topic question before retrieval; when `false`, a deterministic doc-meta-stripped fallback query is used for every heading (fail toward honesty — never an error). Default preserves the built behavior. |
| `DOCBRAIN_GENERATE_VERIFY_CROSS_CHECK` | `true` | Enable the per-claim **live cross-check** (the moat core): each grounded claim is re-verified against the live system it came from. When `false` (or the budget below is `0`), the cross-check is skipped and claims are not live-verified — they default to **NotFound**, never **Contradicted/stale** (the safety invariant: absence is never a stale flag). |
| `DOCBRAIN_GENERATE_VERIFY_CROSS_CHECK_BUDGET_SECS` | `120` | Per-document wall-clock budget (seconds) for the live cross-check. `0` is a disable sentinel (same honest path as `DOCBRAIN_GENERATE_VERIFY_CROSS_CHECK=false`). The cross-check's internal per-call and probe timeouts are fixed safety rails, not tunable. |
| `DOCBRAIN_GENERATE_VERIFY_BROAD_PROBE_TOP_K` | `12` | `top_k` for the two independent non-agentic corpus probes (broad hybrid + catalog title) that confirm a *false* gap before a section degrades to `NEEDS INPUT`. Floored at `1` (a `0`-`top_k` probe returns no candidates). Default `12` (parity with the per-section ask candidate set). |
| `DOCBRAIN_GENERATE_VERIFY_REVIEW_MAX_ROUNDS` | `2` | Max send-back rounds for the post-assess review gate that re-aims a stale section back to write. **Hard-capped at `2`** (the documented maximum); a value `>2` is silently clamped to `2`. `0` disables send-back rewrites (stale claims still ship with a loud inline marker). |
| `AUTOPILOT_APPLIED_WRITES_RETENTION_DAYS` | `30` | Days the record of what an autopilot write replaced (what a revert needs) is kept, before the nightly prune deletes it. |
| `DOCBRAIN_GENERATE_MAX_CONCURRENT_SECTION_ASKS` | `16` | Most section questions in flight at once across all running `generate` requests and the nightly autopilot loop. Held between 8 and 128. Raise it for generate throughput at the cost of load on the retrieval path. |
| `DOCBRAIN_GENERATE_REFUSE_MIN_ONTOPIC` | `3` | `generate` refuses to write a draft when fewer than this many supported claims are about the topic asked. Not applied when you gave a source to start from or merged into an existing page, or when the ask has no distinctive term. |
| `AUTOPILOT_TREND_V2_ENABLED` | `true` | Use the five-state gap trend (improving, stable, worsening, no gaps open, insufficient signal). Only `true` (any case) or `1` keeps it on; any other non-empty value goes back to the older three-state trend. An empty value keeps it on. |
| `AUTOPILOT_TREND_MIN_EVENTS` | `5` | Fewest gap events (new plus resolved, over four weeks) before a trend is reported. Below it the trend reads insufficient signal. |
| `AUTOPILOT_TREND_IMPROVING_THRESHOLD` | `0.75` | Resolved-to-new ratio at or above which the trend reads improving. |
| `AUTOPILOT_TREND_WORSENING_THRESHOLD` | `0.40` | Resolved-to-new ratio below which the trend reads worsening. Between this and the improving threshold it reads stable. |
| `VERIFY_CORPUS_MIN_SCORE` | `45.0` | Search score above which a gap labelled as a missing document is corrected to poor coverage, because the corpus does have something on it. Read at each check, so it can be tuned without a restart. |

> **Small teams / dev environments:** Set `AUTOPILOT_CRITICAL_USERS=1`, `AUTOPILOT_CRITICAL_SIGNALS=3`, `AUTOPILOT_CRITICAL_THRESHOLD=0.3` to see critical gaps with minimal signal. See [autopilot.md](autopilot.md) for a full tuning guide.

### Improvement evidence

| Variable | Default | Description |
|---|---|---|
| `IMPROVEMENT_REINGEST_CONFIRM_TIMEOUT_HOURS` | `72` | Hours after a fix is published before one whose re-ingest was never confirmed is shown as stale instead of as still in flight. |

### Draft Publishing — Closing the Feedback Loop

When a draft is reviewed and approved, DocBrain can publish it directly to your documentation system, then automatically re-ingest the new page so it immediately improves Q&A answers.

**How it works:**
1. Users ask questions → DocBrain detects a gap (unanswered / low confidence)
2. Autopilot clusters related queries and generates a draft using its memory layers:
   - **Layer 2 (Episodic):** User feedback notes and failed answers inform what was missing
   - **Layer 3 (Semantic):** Knowledge graph entities provide verified org context
   - **Layer 4 (Procedural):** Retrieval rules steer search toward relevant spaces
3. Admin reviews draft → clicks **Publish** (or auto-publish fires if `AUTOPILOT_AUTO_DRAFT=true`)
4. DocBrain creates the page in Confluence with labels and provenance metadata
5. The gap cluster is automatically marked **resolved**
6. Next ingest cycle picks up the new page → loop closed

**Configuration:**

```yaml
draft_publish:
  target: "confluence"              # "confluence" | "none"
  confluence_space_key: "ENG"       # Confluence space key for new pages
  confluence_parent_page_id: ~      # Optional parent page ID
  auto_ingest_after_publish: true   # Re-ingest after publish so DocBrain learns immediately
```

| Variable | Default | Description |
|---|---|---|
| `DRAFT_PUBLISH_TARGET` | `none` | Where to publish drafts: `confluence`, `github`, `gitlab` or `none` |
| `DRAFT_PUBLISH_CONFLUENCE_SPACE_KEY` | — | Confluence space key (e.g. `ENG`, `SAAS`) |
| `DRAFT_PUBLISH_PARENT_PAGE_ID` | — | Optional Confluence parent page ID |
| `DRAFT_PUBLISH_AUTO_INGEST` | `true` | Re-ingest the published page on next ingest cycle |
| `GITHUB_PUBLISH_PR_LABELS` | `docbrain,auto-generated` | Comma-separated labels added to each pull request a draft opens in the GitHub repository target. |
| `GITHUB_PUBLISH_API_URL` | `https://api.github.com` | GitHub API base URL for that target. Set it for GitHub Enterprise. |
| `GITLAB_PUBLISH_MR_LABELS` | `docbrain,auto-generated` | Comma-separated labels added to each merge request a draft opens in the GitLab repository target. |
| `GITHUB_PUBLISH_TOKEN` | — | GitHub token with contents and pull-request write scope, used when a draft is published to a GitHub repository. Kept separate from the ingest token. Publishing to GitHub needs this and `GITHUB_PUBLISH_REPO`. |
| `GITHUB_PUBLISH_REPO` | — | Repository, as `owner/repo`, that receives published drafts. |
| `GITHUB_PUBLISH_BRANCH` | `main` | Branch the pull request is opened against, or committed to when `GITHUB_PUBLISH_CREATE_PR` is `false`. |
| `GITHUB_PUBLISH_DOCS_PATH` | `docs` | Directory in the repository where published drafts are written. |
| `GITHUB_PUBLISH_CREATE_PR` | `true` | `true` opens a pull request for each draft. `false` commits straight to the branch. |
| `GITLAB_PUBLISH_TOKEN` | — | GitLab token with `api` scope, used when a draft is published to a GitLab repository. Publishing to GitLab needs this and `GITLAB_PUBLISH_PROJECT_ID`. |
| `GITLAB_PUBLISH_BASE_URL` | `https://gitlab.com` | GitLab base URL for that target. Set it for a self-hosted GitLab. |
| `GITLAB_PUBLISH_PROJECT_ID` | — | Project that receives published drafts, as a numeric id or a `namespace/repo` path. |
| `GITLAB_PUBLISH_BRANCH` | `main` | Branch the merge request is opened against, or committed to when `GITLAB_PUBLISH_CREATE_MR` is `false`. |
| `GITLAB_PUBLISH_DOCS_PATH` | `docs` | Directory in the repository where published drafts are written. |
| `GITLAB_PUBLISH_CREATE_MR` | `true` | `true` opens a merge request for each draft. `false` commits straight to the branch. |

**Confluence credentials** (shared with the ingest connector):

```bash
CONFLUENCE_BASE_URL=https://your-org.atlassian.net/wiki   # Cloud
# or
CONFLUENCE_BASE_URL=https://confluence.internal/wiki       # Data Center (v1)
CONFLUENCE_USER_EMAIL=bot@your-org.com                     # Cloud: email for Basic Auth
CONFLUENCE_API_TOKEN=<api-token>                           # Cloud: API token / Data Center: PAT
CONFLUENCE_API_VERSION=v2                                  # v2 (Cloud) or v1 (Data Center)
```

**Publish API:**
```
POST /api/v1/autopilot/drafts/{id}/publish   — Admin only
```
Response:
```json
{ "status": "published", "url": "https://...", "page_id": "123456" }
```

## Freshness Scoring

| Variable | Default | Description |
|---|---|---|
| `FRESHNESS_SCHEDULER_INTERVAL_HOURS` | `24` | How often freshness scores are recalculated (hours) |
| `CONTRADICTION_CHECKS_PER_PASS` | `10` | Max docs checked for contradictions per freshness run (LLM cost; increase for more coverage) |
| `CONTRADICTION_INCLUDE_RECENT_EVENT_DOCS` | `true` | Include recent Slack/PR/Jira docs in the contradiction pass alongside stalest docs |
| `CONTRADICTION_EVENT_DOC_MAX_AGE_DAYS` | `90` | Only event-based docs edited within this many days are eligible for contradiction checks |
| `FRESHNESS_LLM_CALLS_PER_PASS` | `50` | Documents given the content-currency check (a model call each) per freshness run. Raise it for a large corpus: at 50 a day, 10,000 documents take about 200 days to cover. Each call costs tokens in proportion to the document. |
| `FRESHNESS_LINK_CHECKS_PER_PASS` | `20` | Documents whose links are checked (an HTTP HEAD each) per freshness run. Cheap next to the model checks. |
| `FRESHNESS_ENGAGEMENT_V2_ENABLED` | `false` | Add reader engagement (who retrieved and who rated a document) to freshness scoring. A value that is not `true` or `false` logs a warning and leaves it off. |
| `FRESHNESS_ENGAGEMENT_MIN_RETRIEVERS` | `3` | Fewest distinct people who must have retrieved a document in the window before its engagement counts. Must be above zero. |
| `FRESHNESS_ENGAGEMENT_MIN_VOTERS` | `3` | Fewest distinct people who must have rated a document in the window before its engagement counts. Must be above zero. |
| `FRESHNESS_ENGAGEMENT_WINDOW_DAYS` | `180` | Days of retrievals and ratings that count. Older activity is ignored, so a document nobody touches drifts back to neutral. |

### Event-Based Source Types

Source types whose documents are permanent historical records — incident threads, merged PRs, support tickets — never go stale and shouldn't be evaluated for content currency or contradictions. The scorer pins their `time_decay = 100` and skips LLM/link/contradiction passes.

This was a hardcoded list until v1.4; it's now configurable so operators can register custom permanent-record source types (e.g. a homegrown incident system) without rebuilding the image.

| YAML key (under `freshness`) | Default | Description |
|------------------------------|---------|-------------|
| `event_based_spaces` | `[slack_thread, github_pr, github, gitlab_mr, jira, linear, pagerduty, opsgenie, zendesk, intercom, fireflies]` | List of `documents.space` values treated as permanent historical records. Capture sources (`slack_capture`, `github_capture`, `gitlab_capture`) are intentionally NOT in the default — design discussions DO go stale. |

Override in `default.yaml` (or via the helm value `freshness.eventBasedSpaces`) to add custom source types.

### Excluding Documents from Freshness Reports

Documents that are intentionally frozen — archived project pages, retros, historical decision records, reference material — should not be evaluated for freshness. Old isn't the same as wrong. DocBrain detects these from source-system metadata at ingest and skips them in the scorer.

The **Freshness page** in the UI shows excluded counts via "View excluded (N)" in the page header. Excluded docs don't appear in the Total / Outdated / Stale / Review / Fresh rollups — they're not noise in the freshness view.

#### Quick recipe — exclude every doc tagged `retrospective` in Confluence

**Helm-managed deployments** (recommended — no image rebuild):

```yaml
# values.yaml
freshness:
  exclusionRules:
    archived_labels:
      - archived          # defaults
      - historical
      - obsolete
      - deprecated
      - frozen
      - reference
      - retrospective     # ← your addition
```

```sh
helm upgrade <release> <chart> -f values.yaml
```

Then in the DocBrain UI:
1. **Freshness → Reclassify lifecycle** (or `POST /api/v1/freshness/backfill-lifecycle`) — re-derives every auto-managed doc against the new rules. Existing retrospective-tagged docs become archived in seconds.
2. **Freshness → Rescore All** — refreshes the rollup numbers.

Future docs with the tag get caught automatically at ingest. No further action needed.

**Direct config edits** (when not using helm): edit `config/default.yaml`, restart the server pod. Same rule.

**Per-doc override** (just one specific document, not the whole tag):

```sh
curl -X PATCH https://your.docbrain.example/api/v1/documents/{doc_id}/lifecycle \
  -H "Authorization: Bearer $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"status": "archived"}'
```

Or use the row action menu in the UI: **⋯ → Mark archived**. Manual overrides are sticky — they survive future syncs even if the source-system label changes back.

#### How detection works

During Confluence ingestion DocBrain reads each page's labels and (for Confluence Cloud) page status. The lifecycle classifier matches against three independent signal sources — any match marks the doc archived:

| YAML key (under `freshness.exclusion_rules`) | Helm value | Default | What it matches |
|----------------------------------------------|------------|---------|-----------------|
| `archived_labels` | `freshness.exclusionRules.archived_labels` | `[archived, historical, obsolete, deprecated, frozen, reference]` | Source labels, case-insensitive. Confluence page labels match here. |
| `archived_page_statuses` | `freshness.exclusionRules.archived_page_statuses` | `[archived, trashed]` | Confluence Cloud `status` field. |
| `archived_title_patterns` | `freshness.exclusionRules.archived_title_patterns` | `['^Archived ', '^\[ARCHIVED\]', '\(archived\)$']` | Regex against doc title — safety net for un-labeled legacy docs. |

These rules are list-shaped and configured in YAML only (env vars can't represent lists).

#### Which lifecycle status to use

The `PATCH /lifecycle` API and the row action menu accept four values. They all exclude the doc from scoring; pick the one that matches intent so your audit trail stays meaningful:

| Status | Meaning |
|--------|---------|
| `active` | Default. Scored normally. Use this to un-archive a doc. |
| `archived` | Frozen historical record. Old by design. |
| `reference` | Evergreen content (style guides, glossaries). Don't nag, don't decay. |
| `deprecated` | Should eventually be deleted, but kept for now. |

#### Reviewing what's been excluded

Click **View excluded (N)** in the Freshness page header. The modal groups docs by lifecycle status (archived / reference / deprecated), shows the source labels that triggered the classification, and exposes a **Mark active** button per row to un-archive a doc directly. Search filters by title, space, or tag.

### Capture Lifecycle

Captured content (GitHub PRs/issues, GitLab MRs, Slack threads) decays with age. Unlike incident records (Jira, PagerDuty, Zendesk, Zendesk) which are permanent historical events, captures represent architectural decisions, design discussions, and code-review threads that can become outdated as systems evolve.

**Cross-document references:** All captured content (GitHub PRs, GitLab MRs, Slack threads, MS Teams messages) automatically extracts URLs and classifies them as references to other known documents (Jira tickets, PRs, MRs, Confluence pages, Slack threads). These references are stored in the `document_references` table and indexed as `ref_doc_ids` on OpenSearch chunks, enabling the RAG ENRICH phase to pull in related document context at query time.

**Space assignment:** Captured documents are stored under a meaningful space name:
- GitHub captures → `owner/repo` (e.g., `myorg/backend`)
- GitLab captures → `group/project` (e.g., `platform/api`)
- Slack captures → channel name (e.g., `platform-incidents`)

This makes `allowed_spaces` ACL filtering work correctly for captured content — a key scoped to `["myorg/backend"]` will include GitHub captures from that repo.

**Age baseline:** The freshness scorer uses the **original content creation date** — when the PR was opened, when the Slack thread started — not the time DocBrain captured it. This means a PR opened 5 years ago is scored as 5 years old even if it was captured yesterday.

**Re-capture:** Running `@docbrain capture` again on the same thread updates the content (picks up new comments) but preserves the original creation date as the age baseline.

### Secret check on captures

Every capture is checked for secrets before it is saved (see
[What a capture never holds](agents.md#what-a-capture-never-holds)). Two checks run: a fixed check of known secret
shapes, and then the fast model your DocBrain is configured with reads the capture's text and lists any password,
key or token written in it. When it lists one it reads the text twice more, and a value counts when at least two of the
three reads agree. **Every capture's text is sent to the configured fast model's provider for this check.**
Use a local provider if capture text must not leave your network.

| Variable | Default | Description |
|---|---|---|
| `CAPTURE_SECRET_CHECK_TIMEOUT_SECS` | `10` | The model check's whole budget for one text, in seconds (every attempt, the reply and its parsing). The default is about three times the measured 95th-percentile reply time of a hosted fast model; size it at three times your fast model's p95 (a self-hosted model is often slower). When the budget runs out the capture is saved after the fixed check and the model checks it as soon as it can; until then it may be served. If it runs out during the two further reads, the value counts. `0` is refused at startup. |
| `CAPTURE_SECRET_CHECK_MAX_REPLY_TOKENS` | `4096` | The reply budget, in tokens, the model check asks the provider for. Many providers refuse a larger output cap. A reply cut off at the budget fails closed (the text is not accepted on that reply) and does not count toward the capture pause. `0` is refused at startup. |

### Conversation distillation, fragment clustering and CI capture

Distillation turns a captured conversation into a few knowledge fragments, clustering groups related fragments, and CI capture analyses merged pull requests and deployments through the API.

| Variable | Default | Description |
|---|---|---|
| `DISTILLATION_ENABLED` | `true` | Distil captured Slack threads and pull-request discussions into fragments after a capture. Every distilled thread sends its text to the configured model. |
| `DISTILLATION_MAX_CONCURRENT` | `3` | Most distillation model calls at once. Held between 1 and 50. |
| `DISTILLATION_MAX_CONTENT_CHARS` | `8000` | Characters of a conversation sent to the model. Held between 100 and 50000. |
| `DISTILLATION_MAX_FRAGMENTS` | `5` | Most summaries kept per conversation. Held between 1 and 5. |
| `FRAGMENT_CLUSTERING_ENABLED` | `true` | Group related fragments into clusters. When off, the clusters endpoint answers 503. |
| `FRAGMENT_CLUSTER_THRESHOLD` | `0.80` | Cosine similarity at or above which fragments are grouped. |
| `FRAGMENT_MIN_CLUSTER_SIZE` | `3` | Fewest fragments that make a cluster. |
| `FRAGMENT_MIN_SOURCE_DIVERSITY` | `2` | Fewest distinct source types a cluster must span. |
| `FRAGMENT_MAX_PER_CLUSTERING_RUN` | `2000` | Most fragments one clustering run looks at. |
| `FRAGMENT_AUTO_INDEX_THRESHOLD` | — | Removed: every capture is now served at once. The server still starts when it is set and logs one warning naming it. Delete it from your configuration. |
| `CI_ANALYZE_ENABLED` | `true` | Build the CI/CD capture analyzer at start (it uses the configured model). `false` leaves the CI capture endpoints without an analyzer. |
| `FRAGMENT_REVIEW_THRESHOLD` | `0.4` | Summary floor. A DocBrain summary whose model-reported confidence is below this is not stored; everything at or above it is served, labelled. The name is kept for compatibility; nothing is queued for review. |
| `FRAGMENT_MAX_CONTENT_LENGTH` | `10000` | Longest capture accepted, in characters. |
| `FRAGMENT_MAX_PER_AUTHOR_PER_HOUR` | `60` | Captures one author may write in any rolling hour, counted in the database so every replica sees one count. A value below 1 is raised to 1. |

## Memory Consolidation

| Variable | Default | Description |
|---|---|---|
| `CONSOLIDATION_INTERVAL_HOURS` | `6` | How often the memory consolidation job runs (hours) |

## Memory Retention (Production)

DocBrain's episodic memory grows with every user query. Without retention, the `episodes` table accumulates unboundedly.

| Variable | Default | Description |
|---|---|---|
| `EPISODE_RETENTION_DAYS` | `90` | Delete answers (episodes) older than N days; `0` or less keeps them forever. An answer's page (`/answers/<id>`) stops opening the moment its answer passes this age, before the daily prune removes it, and states "Kept until <date>". A pruned answer leaves its answer topic; empty topics are removed hourly. |
| `AUDIT_RETENTION_DAYS` | `0` (disabled) | Delete audit log entries older than N days. |

**Production recommendations:**
- Set `EPISODE_RETENTION_DAYS=90` to cap memory growth. Consolidation distills patterns into procedural rules before pruning, so learning is preserved. Answer pages follow the same window.
- Back up the `episodes` and `procedural_rules` tables before enabling retention.
- OpenSearch episode vectors are pruned separately. If OpenSearch is larger than expected, verify `P1-2` (prune OS episodes on PG delete) is resolved.
- Protect your PostgreSQL with: network-level access controls (VPC/security groups), encrypted backups, and a read-only replica for analytics queries.
- API keys are stored as Argon2 hashes — raw keys cannot be recovered from the database. Rotate compromised keys via the admin API.

## Documentation Analytics

The Documentation Analytics module tracks organizational documentation health over time — how fast gaps are being resolved, knowledge half-life, tribal knowledge percentage, and ROI of documentation investment. Snapshots are taken daily during memory consolidation.

| Variable | Default | Description |
|---|---|---|
| `VELOCITY_MINUTES_SAVED` | `15` | Estimated minutes of engineer time saved per successfully answered query. Used for ROI calculation. |
| `VELOCITY_HOURLY_RATE` | `75` | Hourly engineer cost in USD for ROI calculation. |
| `VELOCITY_BULK_UPDATE_MULTIPLE` | `10.0` | Bulk re-ingest guard for net knowledge velocity. A week whose `docs_updated` count exceeds this multiple of the rolling weekly-update norm is treated as a bulk sweep (e.g. a full re-ingest) and capped to the norm, so it cannot inflate the velocity headline or flip the maintenance trend to "accelerating". Lower it on a corpus with very steady authoring to catch smaller sweeps; raise it if legitimate maintenance bursts are being mistaken for sweeps. **Must be finite and `>= 1.0`** — a value of 0, negative, or NaN collapses the bulk-sweep threshold to 0 (every week misclassified as a sweep) and is rejected at startup with a clear error. |
| `VELOCITY_SUBSTANTIVE_UPDATE_CEILING` | `2000` | Absolute ceiling on a single week's substantive (bulk-excluded) update contribution. Applied after the rolling-norm cap to guard the case where the entire history is inflated and the median itself is poisoned. A genuine week of hand-authored doc updates does not exceed this. **Must be `>= 1`** — a negative value or 0 silently zeroes all substantive updates and is rejected at startup with a clear error. |
| `VELOCITY_ROI_V2_ENABLED` | `true` | Use the current return-on-documentation method: needs enough distinct users, leaves out admins, caps each user's votes and recomputes from live data. `false` goes back to the older raw-count formula, which is not suited to executive reporting. |
| `VELOCITY_ROI_MIN_MINUTES_LOW` | `5` | Low end, in minutes, of the time saved per helpful answer. Together with the high end it sets the range the ROI figure is shown in. |
| `VELOCITY_ROI_MIN_MINUTES_HIGH` | `25` | High end, in minutes, of the time saved per helpful answer. |
| `VELOCITY_ROI_MIN_DISTINCT_USERS` | `3` | Fewest distinct non-admin users with positive feedback in the window before any ROI is shown. Below it the page reads insufficient signal. |
| `VELOCITY_ROI_EXCLUDE_ADMIN` | `true` | Leave admins' votes out of the ROI count, because the people who run the product rate it differently. Set `false` only with a reason. |
| `VELOCITY_ROI_MAX_VOTES_PER_USER` | `10` | Most votes from one user counted in the window, so a single enthusiastic user cannot dominate the figure. |
| `VELOCITY_TRIBAL_V2_ENABLED` | `true` | Use the current tribal-knowledge method: a percentage over real domains, with a signal check. `false` goes back to the older fixed rule of two experts and no check. |
| `VELOCITY_TRIBAL_MAX_EXPERTS` | `2` | A domain with this many experts or fewer counts as tribal. A five-person team might use `1`; a large organisation might use `5`. |
| `VELOCITY_TRIBAL_MIN_DOMAINS` | `3` | Fewest domains with feedback signal before the tribal percentage is shown. Below it the page reads insufficient signal. |
| `VELOCITY_MAINTENANCE_TREND_MIN_SNAPSHOTS` | `4` | Fewest snapshots with real activity (documents created or updated, or gap events) before the maintenance trend gives a verdict. Below it the page reads insufficient signal. |

```yaml
velocity:
  minutes_saved_per_query: 15
  hourly_rate: 75
  bulk_update_multiple: 10.0
  substantive_update_abs_ceiling: 2000
```

### Semantic quality scoring

A background sweep in which the language model rates documents that already passed the structural score on accuracy, completeness, clarity and actionability. Set in the `semantic_quality:` block or by these variables.

| Variable | Default | Description |
|---|---|---|
| `SEMANTIC_QUALITY_ENABLED` | `true` | `false` turns the sweep off. |
| `SEMANTIC_QUALITY_INTERVAL_HOURS` | `24` | Hours between sweeps; the first runs five minutes after the server starts. `0` is accepted and runs sweeps back to back. |
| `SEMANTIC_QUALITY_BUDGET` | `50` | Most documents rated in one sweep, capped at `100` whatever you set. |
| `SEMANTIC_QUALITY_STRUCTURAL_THRESHOLD` | `40.0` | Lowest structural score a document needs before it is rated by the model. |

## Knowledge Stream

The Knowledge Stream proactively detects and pushes intelligence: incident early warnings (multiple users hitting the same issue), expertise gaps (domains with single-point-of-failure risk), and context-aware doc updates.

Set in the `stream:` block of the config file or by these variables. The block also accepts `alert_channel`, which nothing reads: the stream posts no message.

| Variable | Default | Description |
|---|---|---|
| `STREAM_ENABLED` | `false` | `true` runs the stream loop inside the server; the start-up log says `Knowledge stream scheduled every N minutes` or `Knowledge stream disabled`. |
| `STREAM_INTERVAL_MINUTES` | `30` | Minutes between stream runs; `0` runs every minute. |
| `STREAM_INCIDENT_WARNING_MIN_USERS` | `2` | Distinct users who must have asked the same troubleshooting question (or one answered with low confidence) within two hours before an incident early warning is raised. |
| `STREAM_EXPERTISE_GAP_DAYS` | `90` | Days; an expertise gap fires for a domain whose latest attribution is older than this and names two or fewer individuals. |

## Evidence Bundles

See [`docs/evidence.md`](evidence.md) for what a bundle is, the verdict taxonomy, and how to verify one offline. These settings control the server-side journal that bundles are exported from.

| Variable | Default | Description |
|---|---|---|
| `EVIDENCE_ENABLED` | `true` | Master switch for the evidence journal. When `false`, no records are written and `docbrain evidence export` has nothing to export. |
| `EVIDENCE_CHECKPOINT_EVERY_N` | `256` | Emit a signed checkpoint after this many journal records. Clamped to `16..=65536`. |
| `EVIDENCE_CHECKPOINT_EVERY_SECS` | `3600` | Emit a signed checkpoint after this many seconds even if the record count hasn't reached `EVIDENCE_CHECKPOINT_EVERY_N` yet. Not clamped — the writer floors a zero value to 1 second internally rather than rejecting it, so a sub-minute value is valid for testing. |
| `EVIDENCE_RECOVERY_PUBKEY` | — | Hex-encoded Ed25519 public key for the cold recovery key, declared at genesis. **Set this before the journal's first boot** — genesis is created automatically on first write and is immutable afterward; without a recovery key declared at genesis, no compromise can ever be declared in-band. Optional but strongly recommended. |
| `EVIDENCE_TSA_URL` | — | RFC 3161 timestamp authority endpoint. Must be an absolute `http`/`https` URL. Optional, off by default. A down or slow TSA never blocks a checkpoint — anchoring is best-effort. v1 stores the returned token but does not cryptographically validate it (a future release adds validation); an anchor is honestly reported as present-but-unvalidated, never granted a trust tier on that basis alone. |
| `EVIDENCE_WITNESS_DIR` | — | Directory for append-only, dated witness files (`witness-YYYY-MM-DD.jsonl`) your organization publishes independently. Must be an existing, writable directory — validated at startup with a create+delete probe, not just an existence check. Optional, off by default. |

The set of event classes journaled (`fragments`, `reviews`, `decisions`, `premises`, `governance` — all on by default) is YAML-only, matching this codebase's convention for other list-shaped config; there is no env override. `governance` (GDPR erasure records) is written unconditionally regardless of this list.

The **default compliance framework** is `eu-ai-act` — a compiled default (there is no env var to change it), selectable per export via `docbrain evidence export --framework <id>` (the export API field is `profile`). It is the only framework v1 ships; see [`docs/evidence.md`](evidence.md#regulation-agnostic-engine-pluggable-compliance-frameworks).

## Knowledge Health

The engine that finds what is wrong with your knowledge base — a claim that no
longer holds on a page people still read — and publishes each one as a `kf/1`
finding. A run only **observes**: it writes findings and emits
`health.finding.*` events, and nothing is sent anywhere unless you enable an
adapter. Findings are readable at `/health` in the console, over
`GET /api/v1/health/findings`, and from the CLI with `docbrain health findings`.

| Variable | Default | Description |
|---|---|---|
| `HEALTH_ENGINE_INTERVAL_SECS` | `21600` | Seconds between scheduled engine runs — four runs a day, with the first one 60 seconds after boot. Set `0` to turn the scheduler off entirely; the boot log then says so, and findings can still be produced on demand with `POST /api/v1/health/run` (admin). A value between `1` and `59` is raised to `60` with a warning: a whole run takes a database-wide lock and walks every detector, so an interval mistyped as `6` would hold that lock continuously. A malformed value warns, naming the value it rejected, and falls back to the default — a typo turns into a slower notice, never into silence. Two servers on one database need no coordination: the second run is refused by an advisory lock, logged as routine, and changes nothing. |
| `HEALTH_OPEN_CAP` | `10` | The most findings that may be `open` at once, across the organisation. The cap is the product decision, not a performance one — a list nobody can finish is a list nobody reads, so the engine holds the rest at `queued` and promotes them as slots free. Claiming a finding moves it to `in_progress` and frees its slot, so the number of findings *awaiting a decision* stays at the cap; `in_progress` is deliberately uncapped, because only a person can create one and a claim left untouched for 14 days expires back to `open`. Each run also re-contests the open slots: a queued finding that outranks the lowest open one by a clear margin takes its place, and the displaced finding returns to `queued`, never to resolved or dismissed. Minimum `1`. |
| `HEALTH_WINDOW_DAYS` | `30` | How far back a detector looks when counting the retrievals and questions that give a finding its impact. Clamped to `7..=365`. Widening it raises the reach of long-lived pages; narrowing it makes the list follow what people are reading this week. |
| `DOCBRAIN_PUBLIC_URL` | — | Public origin of the console, used to build each finding's deep link (`<origin>/?finding=<id>`). Unset leaves the link relative. A trailing slash is trimmed. |

Findings carry no asker identities — evidence is counts and texts. Who changed
a finding's state is recorded separately and shown to admins only.

## Ollama-specific Notes

When using Ollama as both LLM and embedding provider:

- **System prompt**: DocBrain automatically detects Ollama and uses a simplified grounding prompt optimised for smaller models (7B–13B). Larger models like `mistral-large` or `llama3.1:70b` follow the full prompt fine — no action needed.
- **Large model (e.g. 70B) slowness**: If you use a large model (e.g. `llama3.1:70b`) for quality, "Understanding your question..." and the search phase can be slow because intent classification and query rewriting also use that model when `FAST_MODEL_ID` is unset. **Set `FAST_MODEL_ID` to a small Ollama model** (e.g. `llama3.1:8b`) so that only the final answer uses the 70B; intent and rewrite stay fast. Example: `LLM_MODEL_ID=llama3.1:70b` and `FAST_MODEL_ID=llama3.1:8b`. The main answer (synthesis) still uses the 70B for quality.
- **"Error decoding response body" after 2–3 minutes**: The Ollama HTTP client has a default timeout of 120 seconds. If the 70B model takes longer to generate the full response, the connection is cut and you get a decode error. **Set `OLLAMA_TIMEOUT_SECS=300`** (or `600`) so the client waits long enough for the response. Allowed range: 60–900 seconds.
- **Embedding context limits**: If a chunk exceeds the embedding model's context window (e.g. `nomic-embed-text` has 8192 tokens), DocBrain automatically falls back to per-text embedding with truncation at 16,000 characters. A warning is logged: `Ollama: text truncated from N to 16000 chars`. To avoid truncation, lower `CHUNK_SIZE` (e.g. `1000`) when using Ollama embedding.
- **Recommended `CHUNK_SIZE` for Ollama**: `800`–`1200` characters keeps every chunk well within context limits.

## Image Extraction

Image extraction uses LLM vision to describe images embedded in Confluence pages, making visual content searchable.

| Variable | Default | Description |
|---|---|---|
| `IMAGE_EXTRACTION_ENABLED` | `true` | Set to `false` to skip image processing entirely |
| `IMAGE_MAX_PER_PAGE` | `20` | Maximum images to process per Confluence page |
| `IMAGE_MIN_SIZE_BYTES` | `5120` | Skip images smaller than this (5KB — avoids icons/decorators) |
| `IMAGE_MAX_SIZE_BYTES` | `10485760` | Skip images larger than this (10MB — avoids enormous images) |
| `IMAGE_DOWNLOAD_TIMEOUT` | `30` | HTTP download timeout in seconds |
| `IMAGE_LLM_TIMEOUT` | `120` | LLM vision call timeout in seconds (needs more time than download) |

### Jira (`INGEST_SOURCES=jira`)

Ingests closed/resolved Jira issues with all comments. Only resolved issues are indexed —
open issues are in-flight and contain incomplete information.

| Variable | Default | Description |
|---|---|---|
| `JIRA_BASE_URL` | — | Jira instance URL. e.g. `https://mycompany.atlassian.net` |
| `JIRA_USER_EMAIL` | — | Service account email for Basic auth (Jira Cloud) |
| `JIRA_API_TOKEN` | — | Atlassian API token (generate at id.atlassian.com) |
| `JIRA_PROJECTS` | — | Comma-separated project keys. e.g. `PROJ,INFRA,PLATFORM`. Empty = all projects. |
| `JIRA_JQL_FILTER` | — | Extra JQL clause appended to the default query. e.g. `priority = High` |
| `JIRA_LOOKBACK_DAYS` | `365` | How many days back to ingest resolved issues |
| `JIRA_ISSUE_TYPES` | `Bug,Story,Task,Epic` | Comma-separated issue types to include |

**Default JQL constructed:**
```
project in (PROJ,INFRA) AND issuetype in ("Bug","Story") AND updated >= -365d AND statusCategory = Done ORDER BY updated DESC
```

**Notes:**
- Issue descriptions use Atlassian Document Format (ADF) — DocBrain converts ADF to Markdown automatically, supporting: paragraphs, headings, code blocks, bullet/ordered lists, blockquotes, tables, inline marks (bold, italic, code, links).
- Issue comments are also ADF and are appended after the description.
- Only issues with `resolution != null` are indexed (open/in-progress issues are excluded).
- Jira Server / Data Center also works — use the Data Center base URL with an API token.
- Rate limit: ~10 req/s. DocBrain paginates at 50 issues/page with 120ms between pages.

---

### Slack Threads (`INGEST_SOURCES=slack_thread`)

Ingests Slack threads that carry resolved knowledge — incident postmortems discussed in threads,
architecture decisions, how-to answers, anything marked with a target reaction.

| Variable | Default | Description |
|---|---|---|
| `SLACK_INGEST_TOKEN` | — | Bot token with `channels:history`, `channels:read`, `users:read` OAuth scopes |
| `SLACK_INGEST_CHANNELS` | — | Comma-separated channel names to index (leading `#` optional). Required — an empty list is a startup error; DocBrain never silently ingests every channel a token can see. Private channels need the `groups:read` scope; without it only public channels are listed. e.g. `incident-response,platform,backend` |
| `SLACK_INGEST_MIN_REPLIES` | `3` | Minimum replies for a thread to be indexed (reply volume signals value) |
| `SLACK_INGEST_REACTIONS` | `white_check_mark,bookmark` | Comma-separated reaction names that mark a thread indexable, regardless of reply count |
| `SLACK_INGEST_LOOKBACK_DAYS` | `90` | How far back to look for threads |

**Notes:**
- A thread is indexed if it has `>= SLACK_INGEST_MIN_REPLIES` **or** if the parent message has any of the `SLACK_INGEST_REACTIONS`.
- Threads with only 1 message (the parent, no replies) are always skipped.
- **Cross-document references** are automatically extracted from Slack mrkdwn links (`<url|label>`) and bare URLs in all thread messages — linked PRs, Jira tickets, Confluence pages, and other documents are added to the reference graph.
- Slack API rate limit is Tier 3 (50 req/min) — the source automatically throttles with 1.2s between thread fetches.
- Use `/docbrain capture` slash command in any Slack thread to capture it on-demand outside the scheduled ingest.

---

## Slack Integration

| Variable | Default | Description |
|---|---|---|
| `SLACK_BOT_TOKEN` | — | Slack bot token starting with `xoxb-` |
| `SLACK_SIGNING_SECRET` | — | Slack app signing secret for webhook verification |
| `NOTIFICATION_INTERVAL_HOURS` | `24` | How often health alerts are posted to the alert channel (requires Slack) |
| `NOTIFICATION_SPACE_FILTER` | — | No longer read: it scoped the author DMs about documents, which were retired. Setting it logs a warning |
| `SLACK_GAP_NOTIFICATION_CHANNEL` | — | Slack channel that receives a message when a gap analysis finds new critical gaps. Unset sends none. Needs `SLACK_BOT_TOKEN`. |

### Slack Capture Access Control

The `/docbrain capture` slash command has no restrictions by default — any user in any channel can trigger it. Use these env vars to restrict access:

| Variable | Default | Description |
|---|---|---|
| `SLACK_CAPTURE_ALLOWED_CHANNELS` | — | Comma-separated channel names (without `#`) or channel IDs that are allowed to use `/docbrain capture`. Empty = all channels. e.g. `platform-team,infra-review` |
| `SLACK_CAPTURE_ALLOWED_USERS` | — | Comma-separated Slack usernames or user IDs allowed to trigger `/docbrain capture`. Empty = all users. e.g. `alice,U01234567` |

If a request is rejected, DocBrain responds with an ephemeral message: `⚠️ You don't have permission to use /docbrain capture in this channel.`

Channel check matches against both `channel_name` and `channel_id`. User check matches against both `user_name` and `user_id`.

## Server

| Variable | Default | Description |
|---|---|---|
| `SERVER_PORT` | `3000` | Port the DocBrain API server listens on |
| `SERVER_BIND` | `127.0.0.1` | Address the DocBrain API server binds to. Use `0.0.0.0` in a container so the port is reachable; the Compose file and the Helm chart both set it. |
| `RUST_LOG` | `info` | Log verbosity and filter, in the `tracing` filter syntax: a level (`trace`, `debug`, `info`, `warn`, `error`) or per-module directives such as `docbrain_core=debug,info`. Read by the server and the ingest binary. |
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3001` | Comma-separated allowed CORS origins for the web UI |
| `LOG_FORMAT` | — | `json` writes one JSON object per log line, for log aggregators. Anything else, or unset, writes compact text. Read by the server and by the ingest binary. |
| `NOTIFICATION_RETENTION_DAYS` | `30` | Days the notification feed keeps an entry. A value that is not a whole number above zero is ignored and the default applies. |
| `MAX_QUERY_LENGTH` | `4000` | Longest question or incident description the server accepts. A longer one is refused with a 400. An admin config reload applies a change without a restart. |
| `DOCBRAIN_LICENSE_FILE` | — | Path to a subscription certificate file. When set it takes precedence over a certificate uploaded in the console, and the server warns if the two differ. Unset uses the uploaded one. The certificate gates nothing: DocBrain keeps working either way. |

### Background workers

Each worker runs on its own timer inside the server. Setting an interval to `0` turns that worker off.

| Variable | Default | Description |
|---|---|---|
| `SIGNAL_INGEST_INTERVAL_SECS` | `900` | Seconds between runs of the behavioural-signal ingester. |
| `EXPERTISE_SCORER_INTERVAL_SECS` | `3600` | Seconds between runs of the expertise scorer. |
| `CALIBRATION_REFIT_INTERVAL_SECS` | `604800` | Seconds between refits of the expertise calibration model (the default is one week). The model is also fitted once at start. |
| `UI_GATE_REFRESH_INTERVAL_SECS` | `300` | Seconds between runs of the ownership-accuracy audit that decides whether confident attributions are shown. `0` leaves them hidden for good. A value that is not a whole number logs a warning and the default applies. |
| `DOCBRAIN_ATTESTATION_INTERVAL_SECS` | `604800` | Seconds between identity-attestation snapshots (the default is one week). Each snapshot counts identities per connected source over a fixed 90-day window. |
| `SECRET_SCAN_INTERVAL_SECS` | `300` | Seconds between passes that scan changed documents for secrets. `0` turns the pass off. A value between 1 and 59 is raised to 60; a value that is not a whole number keeps the default. |

### Expertise ownership gate

The thresholds a team must clear before DocBrain publishes it as the owner of a subject. Every one must hold or the scorer abstains. Set in the `expertise_gate:` block or by these variables.

| Variable | Default | Description |
|---|---|---|
| `EXPERTISE_GATE_V_MIN` | `1.0` | Lowest decayed team score that can publish. |
| `EXPERTISE_GATE_N_MIN` | `5` | Fewest raw signals that can publish. Below it the reason reads "fewer than N signals". |
| `EXPERTISE_GATE_M_ASKERS` | `2` | Fewest distinct people who must have asked about the subject. |
| `EXPERTISE_GATE_DIVERSITY_MIN` | `2` | Fewest distinct signal types the evidence must come from. |
| `EXPERTISE_GATE_MARGIN_FRAC` | `0.25` | How far the leading team must beat the second, as a fraction of the second's score. A subject with only one scored team passes this check. |
| `EXPERTISE_GATE_UI_ENABLED` | `false` | Master switch for showing confident ownership to people. While `false` the gate stays closed and ownership is never shown. |
| `EXPERTISE_GATE_UI_MIN_AUDIT_SAMPLES` | `30` | Fewest audited confident attributions before the gate can open; with fewer it stays closed whatever the rate. |
| `EXPERTISE_GATE_UI_CONFIDENTLY_WRONG_BAR` | `0.0` | Highest audited confidently-wrong rate at which confident ownership is shown. At `0.0` only a rate of zero opens it. |

### Reconciler

Finds documents whose search-index entries have drifted from the database and queues them for re-index, and removes index entries of captures whose row is gone. Each cycle is bounded, and a drifted corpus is repaired over several cycles.

| Variable | Default | Description |
|---|---|---|
| `RECONCILER_ENABLED` | `true` | Anything except `false` or `0` leaves the reconciler on, so a typo does not switch off the repair. |
| `RECONCILER_INTERVAL_MINUTES` | `10` | Minutes between reconciler cycles. |
| `RECONCILER_BATCH_LIMIT` | `500` | The most documents or chunks one pass inspects per cycle. |

### Command-line client

Read by the `docbrain` CLI on the machine it runs on, not by the server.

| Variable | Default | Description |
|---|---|---|
| `DOCBRAIN_CORPUS_DROP_ALERT` | `0.10` | The fraction of a corpus that may disappear between two looks at the same server URL before the CLI says so. Beats the `corpus_drop_alert_fraction` key in `~/.docbrain/config.json`. A value outside `0.0` to `1.0` is ignored. |
| `DOCBRAIN_GENERATE_TIMEOUT_SECS` | `900` | How long `docbrain generate` waits for the server, in seconds. Held between 30 and 900; a value that is not a number keeps the default. A long generate against an existing page can take several minutes. |

### Event bus and outbound webhooks

The internal event bus is always on. These settings size it and govern the webhooks DocBrain sends to your own endpoints.

| Variable | Default | Description |
|---|---|---|
| `EVENT_BUS_CAPACITY` | `4096` | Events the bus buffers before a slow listener starts missing some. Held between 16 and 65536. |
| `WEBHOOK_DELIVERY_TIMEOUT_SECONDS` | `10` | Seconds one webhook delivery attempt may take. Held between 1 and 60. |
| `WEBHOOK_CIRCUIT_BREAKER_THRESHOLD` | `10` | Failed deliveries in a row after which a webhook subscription is switched off. Held between 3 and 100. |
| `WEBHOOK_MAX_RETRIES` | `4` | Delivery attempts, counting the first. Held between 1 and 10. |
| `ALLOW_INTERNAL_WEBHOOKS` | `false` | `true` or `1` lets a webhook target localhost or a private address. Off, those targets are refused. Turn it on only when the receiver is on your own network. |

### Governance SLA checker

The periodic check that finds subjects that have breached a space's SLA and announces each one. The SLA limits themselves are stored per space.

| Variable | Default | Description |
|---|---|---|
| `SLA_CHECKER_INTERVAL_HOURS` | `1` | Hours between runs of the checker. |
| `SLA_CHECKER_QUERY_TIMEOUT_SECS` | `30` | Seconds the checker's database transaction may run. A statement that hits it rolls the whole run back. |
| `SLA_CHECKER_MAX_EVENTS_PER_RUN` | `50` | Most breaches announced in one run. The rest are announced on the next run. |

## Rate Limiting

There are three limiters, and none of them has a global switch. What follows
is what the server does, checked against the code on 2026-09-13; an earlier
version of this section described four `RATE_LIMIT_*` variables that nothing
has ever read.

**Per API key.** Every authenticated request is counted against the key's own
`rate_limit_rpm`, set when the key is created (`POST /api/v1/admin/keys`,
default `60`). Login session keys use `SESSION_RATE_LIMIT_RPM` instead (see
Auth / Sessions, default `240`). Over the limit the server answers
`429 Too Many Requests` with `Retry-After: 60`.

**Login protection, per account.** Ten failed logins for one email within
600 seconds lock that email out for the rest of the window. These two numbers
are constants in the server, not configuration.

**Login protection, per IP.** The public auth routes also count failed
attempts per client IP:

| Variable | Default | Description |
|---|---|---|
| `IP_LOGIN_MAX_FAILURES` | `100` | Failed login attempts allowed per IP per window before the IP is answered `429` |
| `IP_LOGIN_LOCKOUT_WINDOW_SECS` | `600` | Length of that window in seconds; also the `Retry-After` value on the `429` |
| `TRUSTED_PROXY_HOPS` | unset (Docker Compose: `1`) | Unset: the client IP is the socket address and `X-Forwarded-For` is ignored. `N` ≥ 1: the number of proxies of yours in front of the server; the IP is the Nth entry from the right of `X-Forwarded-For` (every header field, in order), i.e. the address the farthest of them saw. Each proxy must append the address it saw: Caddy replaces the header for a peer it does not trust, so a load balancer in front of the Compose proxy needs `trusted_proxies` in `config/Caddyfile`, and then `2`. Docker Compose sets `1` for its proxy; one nginx or one ingress controller such as ingress-nginx is also `1`; Google Cloud's HTTP(S) load balancer adds two entries, so `2`. Set this only behind proxies you control, with the server reachable only through them: too high a value lets a caller choose its own IP for the limiter, too low counts every caller as your proxy. |

The webhook endpoints (`/github/events`, `/gitlab/events`, `/confluence/events`,
`/slack/*`, `/ms-teams/events`) are not rate limited; each verifies its
provider's signature or shared secret instead.

## Auth / Sessions

| Variable | Default | Description |
|---|---|---|
| `LOGIN_SESSION_TTL_HOURS` | `720` | Session key lifetime after login (hours). `0` = no expiry. |
| `MAX_PAT_DAYS` | `90` | Longest life, in days, of an API key a person creates for themselves and of an app connection made with Sign-in for apps. The 30-day unused limit on app connections is fixed. |
| `SESSION_RATE_LIMIT_RPM` | `240` | Requests per minute for a login session key. Named tokens keep the `60` default; a browser session needs more, because one page load issues several calls and the auth guard retries. Too low shows up as a UI that looks like an outage. |
| `ADMIN_KEY_FILE` | `/app/admin-bootstrap-key.txt` | Where to write the admin bootstrap key on first boot |
| `IDLE_TIMEOUT_HOURS` | `0` | Hours without use after which an API key is rejected as if expired, whatever its lifetime. `0` turns it off. |
| `BOOTSTRAP_KEY_ENABLED` | `true` | `false`, `0`, `no` or `off` revokes the bootstrap admin key at start and keeps it revoked; if the revoke fails the server refuses to start. Set it once you have created your own admin keys. |
| `BOOTSTRAP_ADMIN_KEY` | — | Pre-set value for the bootstrap admin key on first boot, as the Helm chart's initial-admin secret does. Unset, a key is generated and written to `ADMIN_KEY_FILE`. |

### Access lists at ingest

Which source access lists are carried onto indexed documents, and how often the access state is rechecked. The Confluence mode sits here with the others so one table covers every source.

| Variable | Default | Description |
|---|---|---|
| `ACL_CONFLUENCE_MODE` | `off` | How Confluence page access is carried into answers. `off` records no access list. `mirror` copies each page's Confluence permissions, so people only see pages they can open there. `public` marks every page open to all. `admin_only` marks every page admin-only. A space can override it with `ACL_CONFLUENCE_SPACE_<KEY>` (the space key in capitals). An unknown value logs a warning and counts as `off`. |
| `ACL_GITHUB_MODE` | `off` | How access to GitHub documents (captures, pull requests and other GitHub documents) is carried into answers. Same values as `ACL_CONFLUENCE_MODE`: `off`, `mirror`, `public` or `admin_only`; an unknown value counts as `off`. Needs `GITHUB_TOKEN` or `GITHUB_CAPTURE_TOKEN`; with neither, no access list is read. |
| `ACL_SLACK_MODE` | `off` | How access to Slack documents (threads and captures) is carried into answers. Same values as `ACL_CONFLUENCE_MODE`; an unknown value counts as `off`. Needs `SLACK_BOT_TOKEN`; without it, no access list is read. |
| `ACL_JIRA_MODE` | `off` | How access to Jira issues is carried into answers. Same values as `ACL_CONFLUENCE_MODE`; an unknown value counts as `off`. Needs `JIRA_BASE_URL` and `JIRA_API_TOKEN` (and `JIRA_USER_EMAIL` for Cloud); without them, no access list is read. |
| `GITHUB_CAPTURE_TOKEN` | — | Token used to read who can see a GitHub document during ingest when `GITHUB_TOKEN` is not set. With neither set, no access list is read for GitHub documents. |
| `ACCESS_MEMBERSHIP_SWEEP_SECS` | `240` | Seconds between reads of who belongs to each Confluence space and group and each Slack channel, which access decisions are made from. `0` turns the sweep off. Otherwise a whole number from `60` to `480`; anything else is a configuration error and the server does not start. |
| `ACCESS_STATE_SWEEP_SECS` | `900` | Seconds between reads of who can open each Confluence page and Jira issue. `0` turns the sweep off. Otherwise a whole number from `300` to `1500`; anything else is a configuration error and the server does not start. |

### Access lists at query time

Whether the access lists carried onto documents are applied when a question is answered, and what a person is told when something was filtered out. Set in the `acl:` block or by these variables.

| Variable | Default | Description |
|---|---|---|
| `ACL_MODE` | `off` | Whether answers are filtered by who may read each source. `off` applies no filter. `warn` works out what would be filtered and logs it, but keeps every result, so you can check coverage on a real corpus first. `enforce` drops what the asker may not read. |
| `ACL_UNKNOWN_POLICY` | `deny` | What to do with a chunk that carries no access list: `deny` or `allow`. Under `enforce`, `deny` drops it and `allow` keeps it. |
| `ACL_RECALL_OVERFETCH` | `1.0` | Declared as the factor by which the search `top_k` is raised while the filter is active, but no code reads it yet, so changing it has no effect. |
| `ACL_USER_PRINCIPAL_MAX_AGE_SECS` | `3600` | Seconds after which a person's stored group memberships count as stale and are left out of what they may read. `0` turns this check off. |
| `ACL_DENIAL_MODE` | `disclosed_no_count` | What an answer says when access filtering removed content. `silent` gives generic text and no counts. `disclosed_no_count` says a filter was applied without saying how much. `disclosed` adds the number of chunks filtered. Admins see `disclosed` by default. |
| `ACL_DENIAL_REFERRAL` | — | Who the denial text tells people to contact; it fills `<referral>` in the message. Default `your administrator`. |
| `ACL_DENIAL_AUDIT` | `false` | `true` writes a row to the `acl_audit_log` table whenever a denial removed content, for compliance review. |
| `ACL_DENIAL_AUDIT_RAW_QUERY` | `false` | `true` stores the question text in the audit log. Off, only a SHA-256 hash is stored. Queries can themselves be sensitive, so leave it off unless a compliance rule needs the text. |

### Access-list backfill tool

`docbrain-acl-backfill` copies source-system access lists onto documents that were indexed before access mirroring existed. It walks one source's documents, stores each access list, and updates every chunk in the search index. It is resumable: by default it skips documents that already have an access list. It reads the same `ACL_CONFLUENCE_MODE` setting as ingest and does useful work only when that is `mirror`.

| Variable | Default | Description |
|---|---|---|
| `ACL_BACKFILL_SOURCE` | `confluence` | Which source to backfill. |
| `ACL_BACKFILL_LIMIT` | `0` | Most documents to process in one run. `0` means no limit. |
| `ACL_BACKFILL_FORCE` | `false` | `true` or `1` fetches and overwrites access lists that already exist. |
| `ACL_BACKFILL_SPACE` | — | Limit the run to one space key. Unset or empty walks every space. |

### Restoring removed secrets from a backup

Read by the `secret_restore` admin tool, which puts back text the secret scan removed, using a database restored from an earlier backup. It runs dry unless told to apply, and also needs `DATABASE_URL` for the live database.

| Variable | Default | Description |
|---|---|---|
| `BACKUP_DATABASE_URL` | — | Connection string of the database restored from the backup. Required by the tool; every session it opens on this database is read only. |

## MCP Tool Platform

Master switch for the live-tool orchestrator. When disabled (the
default), the synthesis path is byte-identical to a build without the tool platform: no orchestrator
round-trip, no fast-LLM dispatch, no measurable overhead. Flip to `true`
once `MCP_OAUTH_ENCRYPTION_KEY` and `MCP_MANIFEST_DIR` are configured to
enable live tool fan-out at answer time.

| Variable | Default | Description |
|---|---|---|
| `MCP_TOOLS_ENABLED` | `false` | Master switch. `true` = orchestrator runs after retrieval, injects live-tool blocks into the synthesis prompt. Requires `MCP_OAUTH_ENCRYPTION_KEY` + `MCP_MANIFEST_DIR` to also be configured (else falls back to disabled). |
| `MCP_TOOLS_FANOUT_MAX_CONCURRENT` | `8` | Max concurrent gateway dispatches in a single fan-out pass (clamped 1..=64). Caps how many tool calls are in flight at once so an always-on full-catalog fan-out under many users can't become a spawn storm. The total wall-clock budget is a separate, fixed bound. |
| `MCP_TOOLS_FANOUT_MAX_ADMITTED_CHARS` | `24000` | Max TOTAL chars of live-tool evidence admitted into a single generate fan-out (clamped 1000..=200000). Delta #5 admits one chunk per tool, so a verbose catalog can stuff the writing prompt with huge live evidence (token cost + lost-in-the-middle); this bounds the sum, dropping whole lowest-priority chunks past the budget at chunk boundaries (never truncated mid-chunk). Applies only to live chunks; corpus chunks keep their own budget. |
| `MCP_TOOLS_FANOUT_COOLDOWN_SECS` | `30` | Per-connector cooldown TTL in seconds (`0` disables; clamped 0..=600). When a connector returns a rate-limit (429), timeout, or transient (5xx / transport) failure, it is skipped from dispatch for this TTL ACROSS requests, so the always-on full-catalog fan-out can't re-hammer a flaky / rate-limited connector on every generate + regenerate. Keyed ONLY on the connector's OWN recent failures and CLEARED the moment it succeeds, so a connector that succeeded more recently than it failed is never suppressed (anti-DoS-on-the-good-tool). |
| `DOCBRAIN_SSRF_ALLOW_LOOPBACK_FOR_INTERNAL_SHIMS` | — | **Required to load the shipped in-process shim manifests.** `confluence_rest` and `jira_rest` are served by docbrain-server itself at `http://localhost:${DOCBRAIN_SERVER_PORT}/internal/mcp/*`, so their endpoint host is loopback by design. The manifest validator's SSRF check rejects loopback by default, and without this set both manifests are skipped at boot (`manifest failed to materialize ... is in a disallowed range`) leaving `manifest_count=0`. Set to `1` or `true` to allow **loopback only** — RFC1918, link-local and IMDS (169.254.169.254) addresses stay blocked, and non-loopback hostnames are still resolved and checked at probe time. |
| `MCP_REGISTRY_PUBKEY` | — | Base64-encoded 32-byte Ed25519 public key used to verify the signed registry index and per-manifest signatures. When unset, `/api/v1/admin/mcp/registry*` and `/install-from-registry` return `503` and the server boots normally; admins can still install via the paste/URL endpoint. No default. |
| `MCP_REGISTRY_URL` | `https://registry.docbrain-ai.com/v1/index.json` | URL of the signed registry index. |
| `MCP_REGISTRY_CACHE_PATH` | `/var/lib/docbrain/registry-cache/index.json` | Disk path for the cached registry index. Acts as the Tier 2 fallback when the network fetch fails. |
| `DOCBRAIN_K8S_SECRET_NAME` | — | Kubernetes Secret name embedded in the kubectl command rendered by `/api/v1/admin/mcp/secrets/audit/{id}`. Optional — when unset the rendered command shows a `<set DOCBRAIN_K8S_SECRET_NAME>` placeholder. |
| `DOCBRAIN_K8S_NAMESPACE` | — | Kubernetes namespace for the same audit endpoint. Optional — placeholder when unset. |
| `DOCBRAIN_PUBLIC_BASE_URL` | `http://localhost:3000` | Origin that MCP sign-in (OAuth) uses to build its callback address. Set it to the address clients reach DocBrain at. `DOCBRAIN_WEB_BASE_URL` falls back to this value when it is unset. |
| `MCP_OAUTH_ENCRYPTION_KEY` | — | Base64 key of exactly 32 bytes that encrypts stored per-user tool sign-in tokens. Without it MCP sign-in is disabled and its endpoints answer `503`. Generate one with `openssl rand -base64 32`. |
| `MCP_MANIFEST_DIR` | `/etc/docbrain/manifests` | Directory the tool manifests are read from. If it does not exist, MCP sign-in is disabled and the server logs a warning. The Helm chart sets `/etc/docbrain/mcp-manifests`. |
| `DOCBRAIN_MCP_SECRET_KEY` | — | Base64 key of 32 bytes that encrypts secrets typed into a manifest in the console. Unset, those inline secrets are refused, while secrets taken from the environment keep working. |
| `DOCBRAIN_INTERNAL_MCP_SECRET` | — | Bearer secret the built-in tool shims require from the orchestrator, which calls them on the loopback address; generated at start when unset (a temporary one, with a note in the log). Set it in production so the value survives a restart; otherwise a rolling deploy drops in-flight sessions. |
| `DOCBRAIN_DM_PERSIST_POLICY` | `strict` | What happens to direct-message content returned by a tool (today, Slack). `strict` removes it before it reaches the answer, the cache or memory. `warn` logs it but keeps it, for staging only. `allow` keeps it. An unknown value logs a warning and uses `strict`. |
| `DOCBRAIN_MCP_LOG_RENDERED_BLOCKS` | `false` | `1`, `true`, `yes` or `on` logs how each tool result was framed in the answer prompt: the header, the argument line and the sizes, never the body. For diagnosing how the model was shown a result. Read once at start. |
| `DOCBRAIN_DM_REDACTOR_DEBUG_PROBE` | `false` | Diagnostic only, off by default. `true`, `1`, `yes` or `on` logs a warning for each Slack tool call with the first part of the parsed response and the redaction report. Direct-message text can land in the log while it is on, so turn it on briefly and back off. |
| `DOCBRAIN_OAUTH_EXCHANGE_DEBUG_PROBE` | `false` | Diagnostic only, off by default. `true` logs the full authorize URL on every tool sign-in, and, when a token exchange cannot be decoded, the first part of the provider's raw reply. That reply can hold part of a credential, so turn it on briefly and back off. |

YAML equivalent:

```yaml
mcp_tools:
  enabled: false
  fanout_max_concurrent: 8
  fanout_max_admitted_chars: 24000
  fanout_cooldown_secs: 30
```

### Dynamic tool discovery

For MCP servers that publish a `tools/list` endpoint, DocBrain can auto-populate
the tool catalog instead of requiring every tool to be hand-declared in the
manifest. Add a `tool_discovery` block:

```yaml
id: my_mcp
display_name: My MCP
# ... rest of manifest ...
tools: []                           # may be empty when discovery is dynamic
tool_discovery:
  mode: dynamic                     # default: static — explicit "dynamic" enables auto-discovery
  refresh_seconds: 3600             # poll interval; must be 0 (boot-only) or >= 60
  per_tool_defaults:
    output_size_cap_bytes: 16384    # <= 16384 ceiling
    latency_budget_ms: 7000         # <= 8000 ceiling
```

**Read-only invariant.** DocBrain only registers tools where the upstream
declares `annotations.readOnlyHint == true`. Tools without the hint, or marked
`false`, are silently dropped at probe time. DocBrain does not dispatch write
operations via MCP; this is a platform-wide invariant enforced at three gates:
the probe-time filter, the required `read_only` field on every static tool, and
a final assertion in `eligibility_for_user`.

**Static tool field — `read_only`.** Every entry in `tools:` MUST declare
`read_only: true` (or `false`, which will then be blocked by the D1 gate at
eligibility time). This is a required field; manifests missing it fail to parse.

**Probe credentials.**
- *Service-account or mixed auth*: the manifest's service-account header is used
  for probes. No additional setup required.
- *OAuth-only auth*: an admin must designate a probe user via
  `PUT /api/v1/admin/mcp/manifests/{id}/probe-user`. Until designated, the
  manifest stays in `requires_probe_user` status and serves no tools.

**Static + dynamic name collisions.** When a static tool and a discovered tool
share a name:
- If the static tool has `override_discovered: true`, the static entry wins and
  surfaces with `tool_source: "static_override"`.
- Otherwise BOTH entries are dropped from eligibility and the manifest's
  discovery status flips to `degraded_collisions`. Inspect via
  `GET /api/v1/admin/mcp/manifests/{id}`.

**Boot behaviour.** Dynamic manifests are excluded from eligibility until the
first successful probe completes. Status surfaces in the admin detail endpoint
as `pending` -> `ok` (or `failed` / `requires_probe_user`).

### Rootly on-call shim

The `rootly` manifest is served by an internal shim that exposes two read-only
tools — `rootly.get_oncall` (who is on call now) and `rootly.list_overrides`
(scheduled overrides). Unlike OAuth manifests, the shim authenticates to
Rootly's REST API with an org-level token it reads directly from its own env
(it is not routed through `config/default.yaml`). Set these as env vars (e.g.
in the Kubernetes Secret via `mcpTools.serviceAccount.rootly.*` in Helm):

| Variable | Default | Description |
|---|---|---|
| `ROOTLY_API_TOKEN` | — | Org-level Rootly API token. Required for the on-call shim; when unset the manifest is absent and on-call questions fall back to other sources. Read-only. |
| `ROOTLY_BASE_URL` | `https://api.rootly.com` | Rootly REST API base URL. Override only for self-hosted Rootly. |

## SSO / OIDC

See **[docs/sso.md](sso.md)** for full setup instructions and a provider comparison.

### Generic OIDC (Google, Okta, Auth0, Azure AD, Keycloak, …)

| Variable | Default | Description |
|---|---|---|
| `OIDC_ISSUER_URL` | — | OIDC issuer URL. e.g. `https://accounts.google.com` |
| `OIDC_CLIENT_ID` | — | OAuth2 client ID |
| `OIDC_CLIENT_SECRET` | — | OAuth2 client secret |
| `OIDC_REDIRECT_URI` | — | Redirect URI registered with the provider. e.g. `https://docbrain.co/api/v1/auth/oidc/callback` |
| `OIDC_WEB_UI_URL` | `DOCBRAIN_WEB_BASE_URL`, else `http://localhost:3001` | Where to redirect the browser after a successful login; most deployments leave it unset |
| `DOCBRAIN_OAUTH_SERVER` | `off` (Docker Compose: `on`) | `on` lets apps such as Claude Code connect to the MCP endpoint by signing in and being allowed, instead of a pasted API key. Needs `DOCBRAIN_WEB_BASE_URL` as the one address of console and API (https, or `http://localhost` for development); otherwise it stays off and the log and **Settings › Sign-in for apps** say why. |
| `DOCBRAIN_OAUTH_DOCUMENT_PROXY` | — | Fetch apps' metadata documents through this proxy, which then decides which addresses may be reached. Unset, DocBrain fetches them directly and only from public addresses. |
| `DOCBRAIN_WEB_BASE_URL` | — | Public origin of the DocBrain **web UI**. Drives the MCP-OAuth landing redirect AND the **"view in browser"** deep link the CLI prints after a `generate` (plus the shareable per-document link). Set to the user-facing web origin. With Sign-in for apps on (`DOCBRAIN_OAUTH_SERVER`), the console and the API must both be served at this one origin. Also builds the **Open this answer** link on Slack answers and the **Answer page:** line in IDE answers. Unset → no link is offered (never guessed or hardcoded; the console's own Copy link still works). Trailing slash trimmed. |
| `OIDC_ACCEPT_INVALID_CERTS` | `false` | Set to `true` to skip TLS verification for OIDC discovery. Use for corporate/self-signed CAs. |

### GitHub OAuth 2.0

| Variable | Default | Description |
|---|---|---|
| `GITHUB_CLIENT_ID` | — | GitHub OAuth App client ID |
| `GITHUB_CLIENT_SECRET` | — | GitHub OAuth App client secret |
| `GITHUB_REDIRECT_URI` | — | Callback URL. e.g. `https://docbrain.co/api/v1/auth/github/callback` |

### GitLab OIDC

| Variable | Default | Description |
|---|---|---|
| `GITLAB_OIDC_ISSUER_URL` | — | GitLab instance URL. e.g. `https://gitlab.com` or `https://gitlab.corp.example.com` |
| `GITLAB_CLIENT_ID` | — | GitLab application client ID |
| `GITLAB_CLIENT_SECRET` | — | GitLab application client secret |
| `GITLAB_REDIRECT_URI` | — | Callback URL. e.g. `https://docbrain.co/api/v1/auth/gitlab/callback` |
| `OIDC_ACCEPT_INVALID_CERTS` | `false` | Set to `true` if your self-hosted GitLab uses a corporate/private CA |

### Who may sign in

These settings decide who is allowed to sign in over SSO, not which role they get.

| Variable | Default | Description |
|---|---|---|
| `OIDC_ALLOWED_GROUPS` | — | Comma-separated IdP group names, or GitHub organisations, allowed to sign in. When set, someone in none of them is refused unless an allowed domain matches. With neither this nor `OIDC_ALLOWED_DOMAINS` set, every authenticated user may sign in. |
| `OIDC_ALLOWED_DOMAINS` | — | Comma-separated email domains allowed to sign in, for example `acme.com,acme.org`. A person who matches a domain or a group is let in; with neither list set, everyone is. |
| `OIDC_ALLOW_EMAIL_LINKING` | `false` | `true` or `1` lets a first SSO login attach to an existing local account with the same email, but only when the IdP reports that email as verified. Off, such a login does not take over the account. |

### RBAC Role Mapping

Controls what role new SSO users receive. Roles are re-evaluated on every login.

| Variable | Default | Description |
|---|---|---|
| `OIDC_DEFAULT_ROLE` | `viewer` | Default role for new SSO users: `viewer`, `editor`, or `admin` |
| `OIDC_ADMIN_DOMAIN` | — | Email domain that grants admin. e.g. `mycompany.com` |
| `OIDC_ADMIN_EMAILS` | — | Comma-separated emails that always get admin |
| `OIDC_ADMIN_GROUPS` | — | Comma-separated IdP group names → admin |
| `OIDC_EDITOR_GROUPS` | — | Comma-separated IdP group names → editor |
| `OIDC_ANALYST_GROUPS` | — | Comma-separated IdP group names → analyst (the role between editor and admin: it can moderate answers and fragments) |
| `OIDC_SYNC_ROLE_ON_LOGIN` | `true` | Set `false` or `0` to stop the IdP rewriting an existing user's role at each login, so a role an admin set in DocBrain survives. |

## How to configure DocBrain

1. Copy `.env.example` to `.env`:
   ```bash
   cp .env.example .env
   ```

2. Set the required infrastructure variables:
   - `DATABASE_URL` — PostgreSQL connection
   - `OPENSEARCH_URL` — OpenSearch endpoint
   - `LLM_PROVIDER` + provider-specific API key
   - `EMBED_PROVIDER` + model ID
   - the relevant `sources:` block + its source-specific vars

3. Run migrations:
   ```bash
   sqlx migrate run
   ```

4. Run ingest to populate the knowledge base:
   ```bash
   cargo run --bin docbrain-ingest
   ```

5. Start the server:
   ```bash
   cargo run --bin docbrain-server
   ```

6. On first boot, the admin API key is written to `ADMIN_KEY_FILE` (default: `/app/admin-bootstrap-key.txt`). Save this key — it's displayed once.
