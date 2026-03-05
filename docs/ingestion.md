# Ingestion Guide

DocBrain needs documents to answer questions. This guide walks you through connecting your document sources — Confluence, GitHub, or local files.

## How Ingestion Works

When you run ingestion, DocBrain:

1. **Fetches** documents from your configured source
2. **Converts** them to Markdown (HTML, Confluence storage format, etc.)
3. **Chunks** them using heading-aware splitting (preserves semantic coherence)
4. **Embeds** each chunk into vectors using your configured embedding provider
5. **Indexes** the vectors in OpenSearch for hybrid search (k-NN + BM25)

After ingestion, you can immediately start asking questions. DocBrain cites sources in every answer, linking back to the original document.

## Quick Reference

Configure sources in `config/local.yaml` (gitignored). Put only infrastructure secrets in `.env`.

| Source | `ingest_sources` value | What You Need |
|--------|------------------------|---------------|
| Local files | `local` | A directory of `.md` or `.txt` files |
| Confluence | `confluence` | Atlassian URL, email, API token, space keys |
| GitHub | `github` | Repository URL, optional token for private repos |
| GitHub PRs | `github_pr` | GitHub token, owner/repo |
| GitLab MRs | `gitlab_mr` | GitLab token, project path |
| Slack threads | `slack_thread` | Slack bot token, channel IDs |
| Jira | `jira` | Jira URL, email, API token, project keys |

---

## Option 1: Local Files (Default)

The simplest option. Point DocBrain at a folder of Markdown or text files.

### Setup

Add to `config/local.yaml`:

```yaml
# config/local.yaml
ingest:
  ingest_sources: local
```

And set the path in `.env` (it's a filesystem path, not a secret, but it's deployment-specific):

```env
LOCAL_DOCS_PATH=/data/docs
```

By default, Docker Compose mounts `./examples/sample-docs` to `/data/docs` — so DocBrain works out of the box with the included sample documents.

### Using Your Own Files

**Option A: Edit the volume mount** in `docker-compose.yml`:

```yaml
volumes:
  - /absolute/path/to/your/docs:/data/docs:ro
```

**Option B: Copy files into the sample-docs directory:**

```bash
cp -r ~/my-docs/* examples/sample-docs/
```

### Run Ingestion

```bash
docker compose exec server docbrain-ingest
```

### Supported File Types

- `.md` — Markdown (recommended)
- `.txt` — Plain text

### Verify

```bash
# Ask a question about your docs
docker compose exec server docbrain-cli ask "What is in my documentation?"
```

---

## Option 2: Confluence

Connect DocBrain to your Atlassian Confluence instance. DocBrain fetches pages from the spaces you specify, converts Confluence storage format to Markdown, and indexes everything.

### Step 1: Create a Confluence API Token

1. Go to [https://id.atlassian.com/manage-profile/security/api-tokens](https://id.atlassian.com/manage-profile/security/api-tokens)
2. Click **Create API token**
3. Give it a label (e.g. "DocBrain")
4. Copy the token — you won't see it again

### Step 2: Find Your Space Keys

Space keys are the short identifiers for your Confluence spaces. You can find them in the URL:

```
https://yourcompany.atlassian.net/wiki/spaces/ENG/pages/...
                                              ^^^
                                              This is the space key
```

Common examples: `ENG`, `DOCS`, `OPS`, `PLATFORM`

### Step 3: Configure `config/local.yaml`

```yaml
# config/local.yaml — never committed (gitignored)
ingest:
  ingest_sources: confluence

confluence:
  base_url: https://yourcompany.atlassian.net/wiki
  user_email: you@yourcompany.com
  api_token: your-api-token-here
  space_keys: ENG,DOCS
```

**Multiple spaces**: Separate with commas: `ENG,DOCS,OPS`

**Limiting pages**: By default, DocBrain ingests all pages in each space. To cap the number of pages per space (useful for testing), add:

```yaml
confluence:
  page_limit: 100   # 0 = unlimited (default)
```

### Step 4: Run Ingestion

```bash
# Restart the server to pick up the new config
docker compose restart server

# Run ingestion
docker compose exec server docbrain-ingest
```

You'll see output like:

```
Fetching pages from space ENG... 47 pages found
Fetching pages from space DOCS... 123 pages found
Converting 170 pages to Markdown...
Chunking... 892 chunks created
Generating embeddings... done
Indexing in OpenSearch... done
Ingestion complete: 170 pages, 892 chunks
```

### Step 5: Verify

```bash
docker compose exec server docbrain-cli ask "What are our deployment procedures?"
```

The answer should cite your Confluence pages with links back to the originals.

### Self-Hosted Confluence (Data Center)

DocBrain also supports self-hosted Confluence Data Center 7.x+ instances:

```yaml
# config/local.yaml
confluence:
  api_version: v1
  base_url: https://confluence.yourcompany.com
  api_token: your-personal-access-token
  space_keys: ENG,DOCS
```

**Creating a Personal Access Token (Data Center):**

1. Log in to your Confluence Data Center instance
2. Go to your profile (top-right) > **Settings** > **Personal Access Tokens**
3. Click **Create token**, give it a name (e.g. "DocBrain"), and copy the token

| | Cloud | Self-Hosted (Data Center) |
|---|---|---|
| `CONFLUENCE_API_VERSION` | `v2` (default) | `v1` |
| `CONFLUENCE_BASE_URL` | `https://yourco.atlassian.net/wiki` | `https://confluence.yourco.com` |
| `CONFLUENCE_USER_EMAIL` | Atlassian account email | Not required |
| `CONFLUENCE_API_TOKEN` | API token from Atlassian | Personal Access Token (Bearer auth) |

If your instance uses a self-signed certificate or an internal CA that Docker doesn't trust, disable TLS verification:

```yaml
# config/local.yaml
confluence:
  tls_verify: false
```

Everything else works identically — same space keys, same page limit, same webhook sync, same image extraction.

### Permissions

The API token inherits the Confluence permissions of the user account. DocBrain can only access pages that user can read. For broad access, use a service account with read permissions across your target spaces.

### Troubleshooting

| Problem | Cause | Fix |
|---------|-------|-----|
| `401 Unauthorized` | Wrong email or token | Double-check email matches the Atlassian account that created the token |
| `404 Not Found` | Wrong base URL | Use `https://yourco.atlassian.net/wiki` (must include `/wiki`) |
| 0 pages found | Wrong space key | Check the URL of your Confluence space for the correct key |
| Timeout on large spaces | Too many pages | This is normal for 500+ page spaces — ingestion continues in the background |

---

## Option 3: GitHub Repository

Ingest documentation from a GitHub repository. DocBrain clones the repo, finds Markdown and text files, and indexes them.

### Setup

```yaml
# config/local.yaml
ingest:
  ingest_sources: github

github:
  repo_url: https://github.com/your-org/your-docs-repo
  branch: main
```

**For private repositories**, add a personal access token:

```yaml
github:
  token: ghp_your_token_here
```

### Creating a GitHub Token (for private repos)

1. Go to [https://github.com/settings/tokens](https://github.com/settings/tokens)
2. Click **Generate new token (classic)**
3. Select scope: `repo` (for private repos) or `public_repo` (for public repos only)
4. Copy the token

### Run Ingestion

```bash
docker compose restart server
docker compose exec server docbrain-ingest
```

### What Gets Ingested

DocBrain ingests all `.md` and `.txt` files in the repository. It respects directory structure and uses file paths as metadata for source citations.

### Monorepo?

If your docs are in a subdirectory of a larger repo, DocBrain still ingests the whole repo but filters for documentation files. Future versions will support path filtering.

---

## Image Extraction (Confluence)

When ingesting from Confluence, DocBrain automatically downloads images (diagrams, screenshots, flowcharts) from each page and uses a vision-capable LLM to generate detailed descriptions. These descriptions are injected into the document content and indexed alongside the text — making image content searchable and available for Q&A.

**This is enabled by default.** No extra configuration needed if your LLM provider supports vision.

### How It Works

1. During page processing, DocBrain extracts image references from the HTML
2. Downloads each image attachment from the Confluence API
3. Sends the image to the configured LLM's vision endpoint
4. Injects the description into the Markdown before chunking

### Which Providers Support Vision?

| Provider | Vision Support | Notes |
|----------|---------------|-------|
| AWS Bedrock | Yes | Uses Claude's native vision via Messages API |
| Anthropic | Yes | Uses Claude's native vision via Messages API |
| OpenAI | Yes | Uses GPT-4o vision via Chat Completions API |
| Ollama | Depends on model | Vision models (`llava`, `llama3.2-vision`, `moondream`) work. Text-only models (`llama3.1`) are auto-detected on first call — images are skipped with a warning, no failures. |

### Guardrails

| Guardrail | Value | Reason |
|-----------|-------|--------|
| Max images per page | 20 | Prevent runaway LLM costs on image-heavy pages |
| Min image size | 5KB | Skip icons, avatars, decorative images |
| Max image size | 10MB | Skip huge files |
| Allowed types | `png`, `jpeg`, `gif`, `webp` | Skip PDFs, ZIPs, videos |
| Timeout per image | 30s | Don't block the pipeline |

### Disabling Image Extraction

```env
IMAGE_EXTRACTION_ENABLED=false
```

When disabled, images get a `[Image: filename.png]` placeholder in the text (the pre-existing behavior). You can re-enable later and re-ingest to pick up image descriptions.

### Cost

Image descriptions use the `HAIKU_MODEL_ID` model if set (recommended for cost efficiency), otherwise falls back to `LLM_MODEL_ID`. With Claude Haiku, expect ~$0.001 per image. A full ingestion of 1000 pages with ~3 images each costs roughly $3.

---

## Real-Time Sync: Confluence Webhooks

By default, DocBrain ingests documents when you run `docbrain-ingest` manually or on a cron schedule. But if you want pages to sync **automatically** the moment they're created, updated, or deleted in Confluence, enable webhook integration.

### What It Does

| Confluence Event | DocBrain Action |
|-----------------|-----------------|
| `page_created` | Fetches the new page, chunks it, embeds it, indexes it |
| `page_updated` | Deletes old chunks, re-fetches, re-chunks, re-indexes |
| `page_restored` | Same as created |
| `page_removed` / `page_trashed` | Deletes the page's chunks from OpenSearch and marks it deleted in PostgreSQL |

All processing happens asynchronously — DocBrain returns `200 OK` to Confluence immediately and syncs in the background.

### Step 1: Generate a Webhook Secret

Pick a strong random string. This secret is shared between Confluence and DocBrain for HMAC-SHA256 signature verification.

```bash
# Generate a random secret
openssl rand -hex 32
```

### Step 2: Configure DocBrain

Set the webhook secret as an environment variable (it's a runtime secret injected by the environment):

```env
# .env — webhook secret only
CONFLUENCE_WEBHOOK_SECRET=your-generated-secret-here
```

Confluence credentials must also be set in `config/local.yaml` (DocBrain needs API access to fetch page content when a webhook fires):

```yaml
# config/local.yaml
confluence:
  base_url: https://yourcompany.atlassian.net/wiki
  api_token: your-api-token
  user_email: you@yourcompany.com
```

Restart the server. You should see:

```
[startup] Confluence webhook integration enabled
```

If you see `CONFLUENCE_WEBHOOK_SECRET set but missing CONFLUENCE_BASE_URL/API_TOKEN — webhook sync disabled`, check that both `CONFLUENCE_BASE_URL` and `CONFLUENCE_API_TOKEN` are set.

### Step 3: Configure the Webhook in Confluence

#### Confluence Cloud

1. Go to your Confluence instance → **Settings** (gear icon) → **Webhooks** (under "Atlassian Admin" → find your site)
2. Or use the Atlassian admin: `https://admin.atlassian.com` → your site → **Settings** → **Webhooks**
3. Click **Create webhook**
4. Configure:

| Field | Value |
|-------|-------|
| **URL** | `https://<your-docbrain-domain>/confluence/events` |
| **Secret** | The same secret you set in `CONFLUENCE_WEBHOOK_SECRET` |
| **Events** | Select: `page_created`, `page_updated`, `page_removed`, `page_trashed`, `page_restored` |

5. Save and activate the webhook.

> **Important:** The URL must be HTTPS and publicly reachable from Atlassian's servers. If DocBrain runs behind a firewall, you'll need an ingress or tunnel (e.g., ngrok for testing, or a proper reverse proxy in production).

#### Confluence Data Center (Self-Hosted)

1. Go to **Administration** → **Further Configuration** → **Webhooks** (or install the Webhook plugin if not available)
2. Create a webhook with the same URL and secret as above
3. Select the page events you want to track

### Step 4: Verify

Create or edit a page in Confluence. Within a few seconds, check the DocBrain server logs:

```
[confluence] Processing page_updated for page 'My Test Page' (id=12345)
[confluence] Updated page 'My Test Page' — 8 chunks re-indexed
```

Then ask a question about the content you just changed:

```bash
docbrain-cli ask "What did I just write about?"
```

The answer should reflect the latest content.

### Security

- Every incoming webhook is verified using **HMAC-SHA256** with the shared secret
- The signature is checked via the `X-Hub-Signature: sha256=<hex>` header
- Constant-time comparison prevents timing attacks
- Request body is limited to 1MB
- If verification fails, DocBrain returns `401 Unauthorized` and ignores the event

### Webhooks vs. Scheduled Ingest

| | Webhooks | Scheduled Ingest (`docbrain-ingest`) |
|---|---|---|
| **Latency** | Seconds after page edit | Hours (depends on cron interval) |
| **Scope** | Single page per event | All pages in configured spaces |
| **Use case** | Real-time sync for active teams | Bulk initial load, catch-up, re-indexing |
| **Requirements** | Public HTTPS URL, Confluence webhook config | Just a cron schedule |

**Recommendation:** Use both. Run scheduled ingest as a daily safety net (catches anything webhooks might miss — network blips, downtime), and use webhooks for real-time updates.

---

## Real-Time Capture: `@docbrain capture` and `/docbrain capture`

DocBrain supports on-demand capture from GitHub PRs/issues, GitLab MRs, and Slack threads. Capture **only ingests** the thread into the knowledge base — it does not generate a Q&A reply. After capture, the content is immediately searchable via `/docbrain ask` (Slack) or the API.

### What Capture Does

| Platform | Trigger | What's indexed | Reply |
|----------|---------|----------------|-------|
| GitHub | Comment `@docbrain capture` on any PR or issue | PR/issue description + all comments | Posts a reply comment confirming capture |
| GitLab | Comment `@docbrain capture` on any MR | MR title, description, all human discussion notes | Posts a reply note confirming capture |
| Slack | Run `/docbrain capture` inside a thread | All thread messages, user names resolved | Posts a message in the thread confirming capture |

Capture is separate from `/docbrain ask` (Slack) or `@docbrain ask` (GitHub/GitLab) — those are Q&A commands that answer questions from the knowledge base.

---

## GitHub PR/Issue Capture

Comment `@docbrain capture` on any GitHub pull request or issue to immediately index the discussion.

**Requirements:** GitHub webhook configured to send `issue_comment` and `pull_request_review_comment` events to DocBrain.

### Setup

```env
GITHUB_CAPTURE_WEBHOOK_SECRET=your-webhook-secret   # generate with: openssl rand -hex 32
GITHUB_CAPTURE_TOKEN=ghp_...                         # Personal access token with repo:read scope
```

Optional access control (recommended for shared installations):

```env
GITHUB_CAPTURE_ALLOWED_REPOS=myorg/backend,myorg/frontend  # Only these repos can trigger capture
GITHUB_CAPTURE_ALLOWED_USERS=alice,bob                      # Only these users can trigger capture
```

### Register the Webhook in GitHub

1. Go to your repository: **Settings → Webhooks → Add webhook**
2. Fill in:
   - **Payload URL:** `https://your-docbrain-host/api/v1/github/events`
   - **Content type:** `application/json`
   - **Secret:** same value as `GITHUB_CAPTURE_WEBHOOK_SECRET`
   - **Events:** select `Issue comments` and `Pull request review comments`
3. Save

### What Gets Indexed

- Issue/PR title, description, and all comments
- Threads over 500KB are skipped (DocBrain posts a reply explaining the limit)
- Threads under 200 characters are skipped as too short

### Reply Behavior

On success, DocBrain posts a comment:
```
✅ Captured by DocBrain — 12 chunks indexed and immediately searchable.
This thread will feed Autopilot's next gap analysis run.
```

On failure:
```
⚠️ Capture failed: <error message>
```

### Security and Access Control

- All incoming webhooks are verified via HMAC-SHA256 (`X-Hub-Signature-256` header)
- `GITHUB_CAPTURE_ALLOWED_REPOS` — restrict to specific `owner/repo` pairs
- `GITHUB_CAPTURE_ALLOWED_USERS` — restrict to specific GitHub usernames
- Empty allowlists = all users and repos can trigger capture (acceptable for private org webhooks)

---

## GitLab MR Capture

Comment `@docbrain capture` on any GitLab merge request to immediately index the full MR discussion.

**Requirements:** `GITLAB_CAPTURE_WEBHOOK_SECRET` and `GITLAB_CAPTURE_TOKEN` configured, webhook registered in GitLab.

### Step 1: Configure DocBrain

```env
GITLAB_CAPTURE_WEBHOOK_SECRET=your-webhook-secret   # generate with: openssl rand -hex 32
GITLAB_CAPTURE_TOKEN=glpat-...                       # Personal access token with api scope
GITLAB_CAPTURE_BASE_URL=https://gitlab.com           # Default; set for self-hosted GitLab
```

Optional allowlists (recommended for shared instances):

```env
GITLAB_CAPTURE_ALLOWED_USERS=alice,bob          # Only these users can trigger capture
GITLAB_CAPTURE_ALLOWED_PROJECTS=myorg/myrepo    # Only these projects can trigger capture
```

### Step 2: Register the Webhook in GitLab

1. Go to your project: **Settings → Webhooks**
2. Fill in:
   - **URL:** `https://your-docbrain-host/api/v1/gitlab/events`
   - **Secret token:** same value as `GITLAB_CAPTURE_WEBHOOK_SECRET`
   - **Trigger:** enable **Comments**
3. Click **Add webhook**

### Step 3: Test It

Open any merge request and add a comment containing `@docbrain capture`. Within a few seconds, DocBrain replies with a note on the MR:

```
✅ Captured by DocBrain — 12 chunks indexed and immediately searchable.
This MR will feed Autopilot's next gap analysis run.
```

### What Gets Indexed

- MR title and description
- All human discussion notes (system notes — merge events, label changes, approval events — are excluded)
- Threads over 500KB are skipped silently (too large for the embedding pipeline)

### Reply Behavior

- On success: DocBrain posts a note confirming the chunk count
- On failure: DocBrain posts `⚠️ Capture failed: <error>`
- Replies require `GITLAB_CAPTURE_TOKEN` to be set (token is also used to fetch MR notes)
- The allowlist check is applied to the **commenter** (the user who wrote `@docbrain capture`), not the MR author

### Security and Access Control

- All incoming webhooks are verified via the `X-Gitlab-Token` header (constant-time comparison)
- `GITLAB_CAPTURE_ALLOWED_USERS` — restrict to specific GitLab usernames (the commenter, not the MR author)
- `GITLAB_CAPTURE_ALLOWED_PROJECTS` — restrict to specific project paths (e.g. `myorg/myrepo`)
- If no allowlists are configured, any user in any project can trigger capture — consider setting `GITLAB_CAPTURE_ALLOWED_PROJECTS` at minimum

---

## Slack Thread Capture

Run `/docbrain capture` inside any Slack thread to immediately index the conversation.

**Note:** `/docbrain capture` only ingests the thread. Use `/docbrain ask <question>` separately to query the knowledge base.

### Setup

Ensure the Slack bot is installed and `SLACK_BOT_TOKEN` is configured. The bot needs `channels:history` and `users:read` OAuth scopes.

### Usage

1. Open a Slack thread with a substantive discussion
2. Run `/docbrain capture` inside the thread (not on a top-level message)
3. DocBrain fetches all messages, resolves user names, and indexes the conversation

Within ~15 seconds, DocBrain posts back in the thread:
```
✅ Thread from #platform-incidents captured into DocBrain (8 chunks indexed).
It's now searchable and will be used by Autopilot's next gap analysis.
```

### Access Control

By default, any user in any channel can run `/docbrain capture`. Restrict access with:

```env
SLACK_CAPTURE_ALLOWED_CHANNELS=platform-team,infra-review  # channel names (no #) or IDs
SLACK_CAPTURE_ALLOWED_USERS=alice,U01234567                 # usernames or user IDs
```

- Channel check matches against both `channel_name` and `channel_id`
- User check matches against both `user_name` and `user_id`
- If rejected, DocBrain responds with an ephemeral message: `⚠️ You don't have permission to use /docbrain capture in this channel.`

### What Gets Indexed

- All thread messages with resolved display names and timestamps
- Threads under 200 characters are skipped as too short
- The thread is immediately searchable after capture

---

## Capture Lifecycle and Freshness

### Space Assignment

Captured content is stored under a meaningful space name derived from the source:

| Source | Space assigned |
|--------|---------------|
| GitHub PR/issue | `owner/repo` (e.g., `myorg/backend`) |
| GitLab MR | `group/project` (e.g., `platform/api`) |
| Slack thread | Channel name (e.g., `platform-incidents`) |

This means `allowed_spaces` ACL filtering works as expected — setting `allowed_spaces: ["platform-incidents"]` on an API key will correctly scope answers to Slack captures from that channel, GitHub captures from a matching repo, etc.

### Staleness and Time Decay

Unlike incident records (Jira, PagerDuty, Zendesk), which are permanent historical events, **captured content decays with age**. A GitHub PR discussing an architecture from 5 years ago, or a Slack thread about a since-replaced system, should score low in freshness — not be treated as always-current.

- The freshness scorer uses the **original content creation date** (when the PR/MR was opened, when the Slack thread started) as the age baseline — not the time DocBrain captured it.
- Captures age through the standard time-decay curve: a 2-year-old architectural discussion will score significantly lower freshness than a recent one, which reduces its weight in RAG retrieval and Autopilot gap analysis.
- **Re-capturing the same thread** (running `/docbrain capture` again on the same PR or Slack thread) updates the content but preserves the original creation date as the age baseline.

This ensures that outdated design decisions, replaced architectures, or deprecated processes are progressively de-emphasized in answers as they age — without ever being deleted (the historical record is preserved for explicit search).

---

## Re-Ingestion and Updates

### Updating Documents

Run ingestion again to pick up changes:

```bash
docker compose exec server docbrain-ingest
```

DocBrain uses upsert logic — new and changed documents are updated, unchanged documents are skipped. This is safe to run repeatedly.

### Scheduled Ingestion (Kubernetes)

The Helm chart includes a CronJob that runs ingestion on a schedule:

```yaml
# In values.yaml
ingest:
  schedule: "0 */6 * * *"  # every 6 hours
```

### Full Re-Index

If you change your embedding provider (e.g., from Ollama to OpenAI), you need a full re-index because embedding dimensions differ between providers. Delete the OpenSearch index and re-ingest:

```bash
# Delete the existing index
curl -X DELETE http://localhost:9200/docbrain_chunks

# Re-ingest everything
docker compose exec server docbrain-ingest
```

---

## Multiple Sources

DocBrain supports ingesting from multiple sources simultaneously. Set `ingest_sources` in `config/local.yaml` to a comma-separated list of sources, and configure credentials for each:

```yaml
# config/local.yaml
ingest:
  ingest_sources: confluence,github_pr,jira

confluence:
  base_url: https://acme.atlassian.net/wiki
  user_email: you@acme.com
  api_token: ATATT3x...
  space_keys: DOCS,ENG

github_pr:
  token: ghp_...
  repo: acme/platform

jira_ingest:
  base_url: https://acme.atlassian.net
  user_email: you@acme.com
  api_token: your-jira-token
  projects: ENG,OPS
```

Then run a single ingestion pass to pull from all sources:

```bash
docker compose exec server docbrain-ingest
```

Documents from different sources coexist in the same index and are searched together.

---

## Next Steps

- [Configuration Reference](./configuration.md) — all ingestion-related environment variables
- [Provider Setup](./providers.md) — configure embedding providers for ingestion
- [Architecture](./architecture.md) — how the ingestion pipeline works under the hood
