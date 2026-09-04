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
# Bare metal — default (uses ./config/)
./docbrain-server

# Docker with mounted config dir
docker run -e DOCBRAIN_CONFIG_DIR=/etc/docbrain -v /host/config:/etc/docbrain docbrain-server

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

## LLM Provider

Set `LLM_PROVIDER` to choose your LLM backend. Supported providers: `anthropic`, `openai`, `bedrock`, `ollama`, `groq`, `openrouter`, `together`, `deepseek`, `mistral`, `xai`, `gemini`, `azure_openai`, `cohere`.

| Variable | Provider | Description |
|---|---|---|
| `LLM_PROVIDER` | All | Which LLM to use (see list above) |
| `LLM_MODEL_ID` | All | Model identifier (e.g. `claude-sonnet-4-5-20250929`, `gpt-4o`). Defaults to a provider-appropriate model if not set. |
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

### Confluence

| Variable | Default | Description |
|---|---|---|
| `CONFLUENCE_BASE_URL` | — | Full Confluence base URL. e.g. `https://your-org.atlassian.net/wiki` |
| `CONFLUENCE_USER_EMAIL` | — | Email of the Confluence user (Cloud only — not used for Data Center) |
| `CONFLUENCE_API_TOKEN` | — | API token (Cloud) or Personal Access Token (Data Center) |
| `CONFLUENCE_SPACE_KEYS` | — | Comma-separated space keys to ingest. e.g. `DOCS,ENG,PLATFORM`. Optional when `CONFLUENCE_PAGE_IDS` is set. |
| `CONFLUENCE_PAGE_IDS` | — | Comma-separated page IDs to ingest, **including all descendants**. Pages can be from different spaces. e.g. `421856743,41912006`. Can be used alone or together with `CONFLUENCE_SPACE_KEYS`. |
| `CONFLUENCE_API_VERSION` | `v2` | `v2` for Confluence Cloud, `v1` for self-hosted Data Center |
| `CONFLUENCE_PAGE_LIMIT` | `0` | Max pages to ingest per space. `0` = unlimited |
| `CONFLUENCE_TLS_VERIFY` | `true` | Set to `false` if using self-signed or internal CA certificates |

**Ingestion modes:**

| Mode | Config | Behaviour |
|------|--------|-----------|
| Whole spaces | `CONFLUENCE_SPACE_KEYS=DOCS,ENG` | All pages in the listed spaces |
| Specific page trees | `CONFLUENCE_PAGE_IDS=421856743,41912006` | Listed pages + every descendant, from any space |
| Both | `CONFLUENCE_SPACE_KEYS=DOCS` + `CONFLUENCE_PAGE_IDS=421856743` | Spaces first, then any page trees not already covered |

### GitHub PR Reviews (`INGEST_SOURCES=github_pr`)

Ingests merged pull request descriptions and review comments as searchable documents.
This captures the architectural "why" that lives in code review threads.

| Variable | Default | Description |
|---|---|---|
| `GITHUB_PR_TOKEN` | — | GitHub personal access token with `repo:read` (or `public_repo`) scope |
| `GITHUB_PR_REPO` | — | Repository in `owner/repo` format. e.g. `acme/platform` |
| `GITHUB_PR_LOOKBACK_DAYS` | `365` | How many days back to ingest merged PRs |
| `GITHUB_PR_MIN_COMMENTS` | `1` | Minimum total review comments required to index a PR. Set higher (e.g. `3`) to skip trivial PRs. |
| `GITHUB_PR_LABELS` | — | Comma-separated label filter. Only PRs with at least one matching label are indexed. Empty = index all. |
| `GITHUB_PR_API_URL` | `https://api.github.com` | Override for GitHub Enterprise deployments |

**Notes:**
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

A 500KB content size guard is applied to all capture requests. If a PR or issue thread exceeds this limit, DocBrain posts a reply explaining that the thread was too large to capture.

---

### GitLab MR + Review Notes (`INGEST_SOURCES=gitlab_mr`)

Ingests merged merge request descriptions and discussion threads. Works with gitlab.com and self-hosted GitLab instances.

| Variable | Default | Description |
|---|---|---|
| `GITLAB_MR_TOKEN` | — | GitLab personal or project access token with `api` scope |
| `GITLAB_MR_BASE_URL` | `https://gitlab.com` | Base URL for self-hosted GitLab (e.g. `https://gitlab.mycompany.com`) |
| `GITLAB_MR_PROJECT_IDS` | — | Comma-separated project paths or IDs. e.g. `acme/platform,acme/backend` |
| `GITLAB_MR_LOOKBACK_DAYS` | `365` | How many days back to ingest merged MRs |
| `GITLAB_MR_MIN_NOTES` | `1` | Minimum discussion notes to index an MR. Set higher to skip trivial MRs. |
| `GITLAB_MR_LABELS` | — | Comma-separated label filter. Empty = index all. |
| `GITLAB_MR_TLS_VERIFY` | `true` | Set to `false` to skip TLS certificate verification for self-hosted with internal CA |

**Notes:**
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
| `GITHUB_REPO_URL` | — | GitHub repository URL. e.g. `https://github.com/your-org/your-docs` |
| `GITHUB_TOKEN` | — | GitHub personal access token (for private repos) |
| `GITHUB_BRANCH` | `main` | Branch to ingest |

## Chunking

Controls how documents are split into chunks for embedding. See **[docs/chunking.md](chunking.md)** for tuning guidance and re-ingest instructions.

| Variable | Default | Description |
|---|---|---|
| `CHUNK_SIZE` | `1500` | Target chunk size in characters |
| `CHUNK_OVERLAP` | `200` | Overlap between adjacent paragraph-split chunks |

## RAG Pipeline

| Variable | Default | Description |
|---|---|---|
| `RAG_CLAIM_VERIFICATION` | `true` | Before returning an answer, check any file paths it cites against the file listings recorded when your git sources were ingested. A path that has moved is corrected; one that has been deleted is flagged, dated to the commit the listing came from. Deterministic — no extra LLM call — and inert until a git source has recorded a listing. |
| `PREMISE_MONITOR_ENABLED` | `true` | Enable a standing monitor that re-verifies file-path premises in captured knowledge fragments against your connected sources' file listings every 300 seconds. Fragment-index events trigger extraction and initial verification of the new fragment's premises immediately, rather than waiting for the next sweep; re-verification of existing premises happens on the 300-second sweep. The monitor is fully deterministic — no LLM calls, no extra network requests in the check path. A premise verified at capture that later disappears fires a `premise.broken` event (webhook-subscribable, event-log persisted); restoration fires `premise.restored`. Premises never verified at capture remain `dormant` and never alert. Note: `uncheckable` means "no connected source can currently speak to this premise", not "broken" — it never triggers an alert. |
| `RAG_CACHE_TTL_HOURS` | `24` | How long to cache semantically identical answers (hours) |
| `RAG_CACHE_THRESHOLD` | `0.95` | Cosine similarity threshold for answer cache hits (0.0-1.0) |
| `RAG_CONTEXT_WINDOW_BUDGET_BYTES` | `65536` | Total byte budget for expanded **document** context across all documents; overflow is trimmed from the lowest-ranked end. The real limit is your model's *token* window (~3.2 bytes/token measured), so lower it if your model's window is small — and watch the `stage="rag.staged.context_window"` log line, because providers truncate an over-long prompt silently rather than erroring. |
| `RAG_LIVE_DATA_BUDGET_BYTES` | `32768` | **Aggregate** ceiling on the LIVE DATA block — the concatenation of *every* MCP tool block spliced into one synthesis prompt. Each tool's own output is already capped by `DOCBRAIN_MCP_DEFAULT_TOOL_OUTPUT_CAP_BYTES` (default 32768), so this setting is what bounds **several tools answering at once**; nothing else does. Measured 2026-09-04: a Confluence tool returned 103,479 bytes and reached the prompt as 8,475 after per-tool capping and redaction, so one tool rarely approaches this ceiling. Over budget the block is truncated with an explicit marker and a `warn` log (`live tool output exceeds the synthesis budget`), because a provider does not reject an over-long prompt — it drops the overflow silently. |
| `RAG_TOP_K` | `10` | Chunks retrieved per query. Higher = more context passed to LLM, higher token cost. |
| `RAG_MAX_PER_SOURCE` | `3` | Max chunks per **source group** in the final top-k, where the group is the chunk's `space`: the space key for Confluence (`GOL`, `ENG`, …) and the literal `local` for local-file ingest — *not* the `source_type`. A distribution cap: it stops one group crowding out the others. **Skipped entirely when fewer than two groups are eligible** — with a single group (local-only, one-space Confluence, …) it would otherwise cap total LLM context at 3 chunks regardless of `RAG_TOP_K`, which starves synthesis and presents as a weak model rather than a truncated context. With two or more groups it binds normally, and it may then leave the result under `RAG_TOP_K` by design: two groups at the default of 3 yield at most 6 chunks even with `RAG_TOP_K=10`. Raise this value if you want a small number of groups to fill top-k. |
| `RAG_MAX_PER_DOCUMENT` | `2` | Max chunks from any one **document** in the final top-k. Bounds redundancy: consecutive chunks of one document are frequently near-duplicates. Never relaxed — unlike the per-source cap, this is not about distribution across sources. |
| `RAG_BM25_BOOST` | `1.0` | BM25 keyword search weight relative to vector search. Raise (e.g. `2.0`) for corpora with lots of exact-match queries (error codes, tool names). |
| `SEARCH_MIN_SCORE` | `0.0` | Drop retrieved chunks below this relevance score. `0.0` keeps everything; `0.3`–`0.4` filters low-signal noise. |
| `OPENSEARCH_INDEX` | `docbrain-chunks` | Name of the document chunk index in OpenSearch |
| `OPENSEARCH_EPISODE_INDEX` | `docbrain-episodes` | Name of the episode (Q&A history) index in OpenSearch |

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
| `rag.confidence_gate` | `0.40` | Below this, sources are hidden entirely (answer shown as "general knowledge") | Sources render on low-confidence answers that may mislead users |
| `rag.strong_answer_floor` | `0.55` | Below this, the answer carries a "low confidence" disclaimer | UI stops warning users about borderline matches |

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

> **Small teams / dev environments:** Set `AUTOPILOT_CRITICAL_USERS=1`, `AUTOPILOT_CRITICAL_SIGNALS=3`, `AUTOPILOT_CRITICAL_THRESHOLD=0.3` to see critical gaps with minimal signal. See [autopilot.md](autopilot.md) for a full tuning guide.

### Draft Publishing — Closing the Feedback Loop

When a draft is reviewed and approved, DocBrain can publish it directly to your documentation system, then automatically re-ingest the new page so it immediately improves Q&A answers.

**How it works:**
1. Users ask questions → DocBrain detects a gap (unanswered / low confidence)
2. Autopilot clusters related queries and generates a draft using all 5 memory layers:
   - **Layer 2 (Episodic):** User feedback notes and failed answers inform what was missing
   - **Layer 3 (Semantic):** Knowledge graph entities provide verified org context
   - **Layer 4 (Procedural):** Retrieval rules steer search toward relevant spaces
   - **Layer 5 (Freshness):** Stale source docs are flagged so the draft warns readers
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
| `DRAFT_PUBLISH_TARGET` | `none` | Where to publish drafts: `confluence` or `none` |
| `DRAFT_PUBLISH_CONFLUENCE_SPACE_KEY` | — | Confluence space key (e.g. `ENG`, `SAAS`) |
| `DRAFT_PUBLISH_PARENT_PAGE_ID` | — | Optional Confluence parent page ID |
| `DRAFT_PUBLISH_AUTO_INGEST` | `true` | Re-ingest the published page on next ingest cycle |

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

## Memory Consolidation

| Variable | Default | Description |
|---|---|---|
| `CONSOLIDATION_INTERVAL_HOURS` | `6` | How often the memory consolidation job runs (hours) |

## Memory Retention (Production)

DocBrain's episodic memory grows with every user query. Without retention, the `episodes` table accumulates unboundedly.

| Variable | Default | Description |
|---|---|---|
| `EPISODE_RETENTION_DAYS` | `0` (disabled) | Delete episodes older than N days. Set `90`–`180` for production. `0` = keep forever. |
| `AUDIT_RETENTION_DAYS` | `0` (disabled) | Delete audit log entries older than N days. |

**Production recommendations:**
- Set `EPISODE_RETENTION_DAYS=90` to cap memory growth. Consolidation distills patterns into procedural rules before pruning, so learning is preserved.
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

```yaml
velocity:
  minutes_saved_per_query: 15
  hourly_rate: 75
  bulk_update_multiple: 10.0
  substantive_update_abs_ceiling: 2000
```

## Knowledge Stream

The Knowledge Stream proactively detects and pushes intelligence: incident early warnings (multiple users hitting the same issue), knowledge decay risks (popular but stale docs), expertise gaps (domains with single-point-of-failure risk), and context-aware doc updates.

| Variable | Default | Description |
|---|---|---|
| `STREAM_ENABLED` | `false` | Enable the knowledge stream (proactive push intelligence). Opt-in. |
| `STREAM_INTERVAL_MINUTES` | `30` | How often the stream engine scans for incidents, decay, and expertise gaps. |
| `STREAM_INCIDENT_WARNING_MIN_USERS` | `2` | Minimum unique users asking troubleshoot questions about the same service in 2 hours to trigger an incident warning. |
| `STREAM_ALERT_CHANNEL` | — | Slack channel for critical stream alerts (optional). |
| `STREAM_EXPERTISE_GAP_DAYS` | `90` | Days without expert activity before triggering an expertise gap alert. |

```yaml
stream:
  enabled: false
  interval_minutes: 30
  incident_warning_min_users: 2
  alert_channel: ~
  expertise_gap_days: 90
```

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

The **default compliance profile** is `eu-ai-act` — a compiled default (there is no env var to change it), selectable per export via `docbrain evidence export --profile <id>`. It is the only profile v1 ships; see [`docs/evidence.md`](evidence.md#regulation-agnostic-engine-pluggable-compliance-profiles).

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
| `NOTIFICATION_INTERVAL_HOURS` | `24` | How often stale doc notifications are sent (requires Slack) |
| `NOTIFICATION_SPACE_FILTER` | — | Restrict notifications to a specific Confluence space |

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
| `SERVER_HOST` | `0.0.0.0` | Host/interface to bind |
| `LOG_LEVEL` | `info` | Log verbosity: `trace`, `debug`, `info`, `warn`, `error` |
| `CORS_ALLOWED_ORIGINS` | `http://localhost:3001` | Comma-separated allowed CORS origins for the web UI |

## Rate Limiting

DocBrain applies per-IP rate limiting to unauthenticated routes and per-API-key rate limiting to authenticated routes. Rate limiting is enabled by default.

| Variable | Default | Description |
|---|---|---|
| `RATE_LIMIT_ENABLED` | `true` | Set to `false` to disable all rate limiting (not recommended for production) |
| `RATE_LIMIT_RPM` | `60` | Requests per minute per IP on unauthenticated routes |
| `RATE_LIMIT_AUTH_RPM` | `120` | Requests per minute per API key on authenticated routes |
| `RATE_LIMIT_WEBHOOK_RPM` | `30` | Requests per minute per IP on webhook endpoints (`/github/events`, `/gitlab/events`) |

When a rate limit is exceeded, DocBrain returns `429 Too Many Requests` with a `Retry-After` header indicating when the client may retry.

## Auth / Sessions

| Variable | Default | Description |
|---|---|---|
| `LOGIN_SESSION_TTL_HOURS` | `720` | Session key lifetime after login (hours). `0` = no expiry. |
| `ADMIN_KEY_FILE` | `/app/admin-bootstrap-key.txt` | Where to write the admin bootstrap key on first boot |

## MCP Tool Platform

Master switch for the live-tool orchestrator (Plan 4). When disabled (the
default), the synthesis path is byte-identical to pre-Plan-4: no orchestrator
round-trip, no fast-LLM dispatch, no measurable overhead. Flip to `true`
once `MCP_OAUTH_ENCRYPTION_KEY` and `MCP_MANIFEST_DIR` are configured to
enable live tool fan-out at answer time.

| Variable | Default | Description |
|---|---|---|
| `MCP_TOOLS_ENABLED` | `false` | Master switch. `true` = orchestrator runs after retrieval, injects live-tool blocks into the synthesis prompt. Requires `MCP_OAUTH_ENCRYPTION_KEY` + `MCP_MANIFEST_DIR` to also be configured (else falls back to disabled). |
| `MCP_TOOLS_FANOUT_MAX_CONCURRENT` | `8` | Max concurrent gateway dispatches in a single fan-out pass (clamped 1..=64). Caps how many tool calls are in flight at once so an always-on full-catalog fan-out under many users can't become a spawn storm. The total wall-clock budget is a separate, fixed bound. |
| `MCP_TOOLS_FANOUT_MAX_ADMITTED_CHARS` | `24000` | Max TOTAL chars of live-tool evidence admitted into a single generate fan-out (clamped 1000..=200000). Delta #5 admits one chunk per tool, so a verbose catalog can stuff the writing prompt with huge live evidence (token cost + lost-in-the-middle); this bounds the sum, dropping whole lowest-priority chunks past the budget at chunk boundaries (never truncated mid-chunk). Applies only to live chunks; corpus chunks keep their own budget. |
| `MCP_TOOLS_FANOUT_COOLDOWN_SECS` | `30` | Per-connector cooldown TTL in seconds (`0` disables; clamped 0..=600). When a connector returns a rate-limit (429), timeout, or transient (5xx / transport) failure, it is skipped from dispatch for this TTL ACROSS requests, so the always-on full-catalog fan-out can't re-hammer a flaky / rate-limited connector on every generate + regenerate. Keyed ONLY on the connector's OWN recent failures and CLEARED the moment it succeeds, so a connector that succeeded more recently than it failed is never suppressed (anti-DoS-on-the-good-tool). |
| `DOCBRAIN_SSRF_ALLOW_LOOPBACK_FOR_INTERNAL_SHIMS` | unset | **Required to load the shipped in-process shim manifests.** `confluence_rest` and `jira_rest` are served by docbrain-server itself at `http://localhost:${DOCBRAIN_SERVER_PORT}/internal/mcp/*`, so their endpoint host is loopback by design. The manifest validator's SSRF check rejects loopback by default, and without this set both manifests are skipped at boot (`manifest failed to materialize ... is in a disallowed range`) leaving `manifest_count=0`. Set to `1` or `true` to allow **loopback only** — RFC1918, link-local and IMDS (169.254.169.254) addresses stay blocked, and non-loopback hostnames are still resolved and checked at probe time. |
| `MCP_REGISTRY_PUBKEY` | — | Base64-encoded 32-byte Ed25519 public key used to verify the signed registry index and per-manifest signatures. When unset, `/api/v1/admin/mcp/registry*` and `/install-from-registry` return `503` and the server boots normally; admins can still install via the paste/URL endpoint. No default. |
| `MCP_REGISTRY_URL` | `https://registry.docbrain-ai.com/v1/index.json` | URL of the signed registry index. |
| `MCP_REGISTRY_CACHE_PATH` | `/var/lib/docbrain/registry-cache/index.json` | Disk path for the cached registry index. Acts as the Tier 2 fallback when the network fetch fails. |
| `DOCBRAIN_K8S_SECRET_NAME` | — | Kubernetes Secret name embedded in the kubectl command rendered by `/api/v1/admin/mcp/secrets/audit/{id}`. Optional — when unset the rendered command shows a `<set DOCBRAIN_K8S_SECRET_NAME>` placeholder. |
| `DOCBRAIN_K8S_NAMESPACE` | — | Kubernetes namespace for the same audit endpoint. Optional — placeholder when unset. |

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

**Read-only invariant (D1).** DocBrain only registers tools where the upstream
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
| `OIDC_WEB_UI_URL` | `http://localhost:3001` | Where to redirect the browser after a successful login |
| `DOCBRAIN_WEB_BASE_URL` | — | Public origin of the DocBrain **web UI**. Drives the MCP-OAuth landing redirect AND the **"view in browser"** deep link the CLI prints after a `generate` (plus the shareable per-document link). Set to the user-facing web origin — **not** the API host if they differ. Unset → no link is offered (never guessed or hardcoded). Trailing slash trimmed. |
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

### RBAC Role Mapping

Controls what role new SSO users receive. Roles are re-evaluated on every login.

| Variable | Default | Description |
|---|---|---|
| `OIDC_DEFAULT_ROLE` | `viewer` | Default role for new SSO users: `viewer`, `editor`, or `admin` |
| `OIDC_ADMIN_DOMAIN` | — | Email domain that grants admin. e.g. `mycompany.com` |
| `OIDC_ADMIN_EMAILS` | — | Comma-separated emails that always get admin |
| `OIDC_ADMIN_GROUPS` | — | Comma-separated IdP group names → admin |
| `OIDC_EDITOR_GROUPS` | — | Comma-separated IdP group names → editor |

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
