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

| Source | Set `SOURCE_TYPE` to | What You Need |
|--------|---------------------|---------------|
| Local files | `local` | A directory of `.md` or `.txt` files |
| Confluence | `confluence` | Atlassian URL, email, API token, space keys |
| GitHub | `github` | Repository URL, optional token for private repos |

---

## Option 1: Local Files (Default)

The simplest option. Point DocBrain at a folder of Markdown or text files.

### Setup

Your `.env` should have:

```env
SOURCE_TYPE=local
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

### Step 3: Configure `.env`

```env
SOURCE_TYPE=confluence
CONFLUENCE_BASE_URL=https://yourcompany.atlassian.net/wiki
CONFLUENCE_USER_EMAIL=you@yourcompany.com
CONFLUENCE_API_TOKEN=your-api-token-here
CONFLUENCE_SPACE_KEYS=ENG,DOCS
```

**Multiple spaces**: Separate with commas, no spaces: `ENG,DOCS,OPS`

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

DocBrain also supports self-hosted Confluence Data Center 7.x+ instances. Set one extra variable:

```env
CONFLUENCE_API_VERSION=v1
CONFLUENCE_BASE_URL=https://confluence.yourcompany.com
CONFLUENCE_API_TOKEN=your-personal-access-token
CONFLUENCE_SPACE_KEYS=ENG,DOCS
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

```env
CONFLUENCE_TLS_VERIFY=false
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

```env
SOURCE_TYPE=github
GITHUB_REPO_URL=https://github.com/your-org/your-docs-repo
GITHUB_BRANCH=main
```

**For private repositories**, add a personal access token:

```env
GITHUB_TOKEN=ghp_your_token_here
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

Currently, DocBrain supports one source type at a time (set via `SOURCE_TYPE`). To ingest from multiple sources, run ingestion once per source by changing the env var and re-running:

```bash
# First: ingest local docs
SOURCE_TYPE=local docker compose exec server docbrain-ingest

# Then: ingest Confluence
SOURCE_TYPE=confluence docker compose exec server docbrain-ingest
```

Documents from different sources coexist in the same index and are searched together.

---

## Next Steps

- [Configuration Reference](./configuration.md) — all ingestion-related environment variables
- [Provider Setup](./providers.md) — configure embedding providers for ingestion
- [Architecture](./architecture.md) — how the ingestion pipeline works under the hood
