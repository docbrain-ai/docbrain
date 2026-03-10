# Confluence Webhook — Real-Time Page Sync

DocBrain can update its index the moment a Confluence page is created, updated, or deleted — no polling, no waiting for the next scheduled ingest run.

When a webhook fires, DocBrain re-fetches the page content from the Confluence API, re-chunks and re-embeds it, and updates OpenSearch. Delete events remove all chunks for that page. The response is always `200 OK` within milliseconds; all processing happens asynchronously to satisfy Confluence's 30-second timeout.

## Endpoint

```
POST /confluence/events
```

Confluence sends an `X-Hub-Signature: sha256=<hex>` header. DocBrain verifies the HMAC-SHA256 signature against `CONFLUENCE_WEBHOOK_SECRET` before processing any payload.

## Events handled

| Confluence event | DocBrain action |
|---|---|
| `page_created` | Fetch, chunk, embed, index |
| `page_updated` | Re-fetch, re-chunk, re-embed, update index |
| `page_restored` | Same as `page_updated` |
| `page_removed` | Delete all chunks for this page |
| `page_trashed` | Delete all chunks for this page |

All other events (space events, blog posts, etc.) are acknowledged and ignored.

## Prerequisites

The webhook uses the same Confluence API credentials as the ingest job. If you are already running `SOURCE_TYPE=confluence` ingest, `CONFLUENCE_BASE_URL` and `CONFLUENCE_API_TOKEN` are already set — only `CONFLUENCE_WEBHOOK_SECRET` is new.

If you are running webhooks **without** scheduled ingest, all three credentials are required.

## Environment variables

| Variable | Required | Description |
|---|---|---|
| `CONFLUENCE_WEBHOOK_SECRET` | Yes | Shared secret you generate and paste into Confluence. Enables the webhook endpoint. |
| `CONFLUENCE_BASE_URL` | Yes | Full Confluence base URL, e.g. `https://your-org.atlassian.net/wiki` |
| `CONFLUENCE_API_TOKEN` | Yes | API token (Cloud) or Personal Access Token (Data Center) |
| `CONFLUENCE_USER_EMAIL` | Cloud only | Email of the Confluence user (not required for Data Center) |
| `CONFLUENCE_API_VERSION` | No | `v2` for Cloud (default), `v1` for self-hosted Data Center |

## Helm

Add to your `values.yaml`:

```yaml
confluenceWebhook:
  enabled: true
  webhookSecret: "your-random-secret"   # generate with: openssl rand -hex 32

  # Required only if ingest.confluence is not already configured:
  baseUrl: "https://your-org.atlassian.net/wiki"
  email: "docbrain@your-org.com"        # Cloud only
  apiToken: "your-confluence-api-token"
  apiVersion: v2                        # v2 for Cloud, v1 for Data Center
```

Then upgrade:

```bash
helm upgrade docbrain ./helm/docbrain -f values.yaml
```

## Docker Compose / raw env

```env
CONFLUENCE_WEBHOOK_SECRET=your-random-secret
CONFLUENCE_BASE_URL=https://your-org.atlassian.net/wiki
CONFLUENCE_USER_EMAIL=docbrain@your-org.com
CONFLUENCE_API_TOKEN=your-confluence-api-token
CONFLUENCE_API_VERSION=v2
```

## Configuring the webhook in Confluence Cloud

1. Go to **Confluence Settings** → **System** → **Webhooks** (requires Confluence Admin).
2. Click **Create a Webhook**.
3. Set **URL** to `https://your-docbrain-domain/confluence/events`.
4. Set **Secret** to the same value as `CONFLUENCE_WEBHOOK_SECRET`.
5. Under **Page** events, check:
   - Page Created
   - Page Updated
   - Page Removed
   - Page Restored
   - Page Trashed
6. Click **Save**.

Confluence will immediately send a test ping. DocBrain returns `200 OK` for all unrecognised events, so the ping succeeds.

## Configuring the webhook in Confluence Data Center

1. Go to **Administration** → **System** → **WebHooks**.
2. Click **Create a WebHook**.
3. Set **URL** to `https://your-docbrain-domain/confluence/events`.
4. Set **Secret** to the value of `CONFLUENCE_WEBHOOK_SECRET`.
5. Select the same page events listed above.
6. Set `CONFLUENCE_API_VERSION=v1` in your DocBrain environment.

## Verifying it works

After setup, edit a Confluence page and check the DocBrain server logs:

```
INFO Processing page_updated for page 'My Page' (id=12345)
INFO Updated page 'My Page' — 8 chunks re-indexed
```

If you see a `401 Unauthorized` log entry instead, the shared secret does not match — regenerate it and update both sides.

If you see `CONFLUENCE_WEBHOOK_SECRET set but missing CONFLUENCE_BASE_URL/API_TOKEN`, the Confluence credentials are not configured alongside the secret.

## Using webhooks alongside scheduled ingest

Webhooks and scheduled ingest complement each other:

- Scheduled ingest (via `INGEST_SOURCES=confluence`) handles the full crawl on a cron schedule, catching any changes that arrived while the server was down.
- Webhooks keep the index current between scheduled runs.

When both are active, the same `CONFLUENCE_BASE_URL` / `CONFLUENCE_API_TOKEN` credentials are shared. Set `CONFLUENCE_WEBHOOK_SECRET` in addition to your existing ingest config and both will work independently.
