# External Connectors

DocBrain ships with built-in integrations for Confluence, Slack, GitHub, GitLab, Jira, and PagerDuty. But your team's knowledge lives in dozens of systems — internal wikis, ServiceNow, Notion, SharePoint, Zendesk, custom databases, and tools that don't exist yet.

External Connectors let you plug **any knowledge source** into DocBrain by building a lightweight HTTP adapter. DocBrain handles scheduling, retries, circuit breaking, chunking, embedding, and indexing — your connector just serves three endpoints.

---

## Why Build a Connector?

Built-in integrations cover common platforms, but every organization has knowledge locked in systems that no vendor will natively support:

- **Internal tools** — custom wikis, knowledge bases, runbook systems, design doc platforms
- **SaaS products** — ServiceNow, Notion, SharePoint, Zendesk, Guru, Tettra, Slab
- **Databases** — operational data, configuration registries, incident postmortems stored in custom tables
- **Legacy systems** — platforms with proprietary APIs that only your team understands

Without connectors, this knowledge stays siloed — invisible to DocBrain's Q&A, gap detection, quality scoring, and Autopilot. With a connector, it flows through the same pipeline as every other source: chunked, embedded, indexed, quality-scored, and searchable within minutes.

**Key characteristics:**

- **Stateless protocol** — your connector is a simple HTTP server. No SDK, no library dependency, no language requirement.
- **Pull model** — DocBrain calls your connector on a cron schedule. Your connector doesn't need to know DocBrain's address or push data.
- **Incremental sync** — DocBrain passes a `since` timestamp so your connector only returns documents modified since the last sync.
- **Language agnostic** — implement the three endpoints in Python, Go, Node.js, Rust, a shell script — anything that speaks HTTP.

---

## Architecture

```
                     DocBrain Server                          Your Connector
                    ┌──────────────┐                        ┌──────────────┐
                    │              │                        │              │
  Cron fires ──────▶│  Scheduler   │── GET /health ────────▶│  Health      │
                    │              │◀── {"status":"ok"} ────│  Check       │
                    │              │                        │              │
                    │              │── POST /documents/list▶│  List docs   │
                    │              │◀── [{source_id, ...}] ─│  (paginated) │
                    │              │                        │              │
                    │              │── POST /documents/fetch│  Fetch full  │
                    │              │◀── [{content, ...}] ───│  content     │
                    │              │                        │              │
                    │  ┌─────────┐ │                        └──────────────┘
                    │  │ Ingest  │ │
                    │  │ Pipeline│ │   chunk → embed → index → score
                    │  └─────────┘ │
                    └──────────────┘
                           │
                    ┌──────┴──────┐
                    │ OpenSearch  │  Searchable via Q&A, Autopilot,
                    │ PostgreSQL  │  governance, knowledge graph
                    └─────────────┘
```

**The flow:**

1. DocBrain's scheduler fires based on the connector's cron expression (e.g., `0 */6 * * *` = every 6 hours)
2. **Health check** — `GET /health` to verify the connector is alive
3. **List documents** — `POST /documents/list` with `since` timestamp for incremental sync. DocBrain paginates automatically.
4. **Fetch documents** — `POST /documents/fetch` in batches of 50 (configurable). DocBrain sends source IDs, connector returns full content.
5. **Ingest** — each document is chunked, embedded, and indexed into OpenSearch through DocBrain's standard pipeline
6. **Done** — documents are immediately searchable, visible in governance dashboards, and available to Autopilot

---

## Connector Protocol

Your connector must implement three HTTP endpoints. All communication is JSON over HTTP(S).

### 1. Health Check

```
GET {base_url}/health
Authorization: {auth_header}    # if configured
```

**Response:**

```json
{
  "status": "ok",
  "connector_name": "servicenow-kb",
  "version": "1.0.0"
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `status` | string | yes | `"ok"` or `"error"` |
| `connector_name` | string | no | Human-readable name for logs |
| `version` | string | no | Connector version for debugging |

DocBrain calls this before every sync. If it fails, the sync is skipped and the failure is counted toward the circuit breaker threshold.

---

### 2. List Documents

```
POST {base_url}/documents/list
Authorization: {auth_header}
Content-Type: application/json
```

**Request:**

```json
{
  "since": "2024-03-15T10:30:00Z",
  "page": 1,
  "page_size": 100
}
```

| Field | Type | Description |
|---|---|---|
| `since` | string (RFC 3339) | Only return documents modified after this timestamp. `null` on first sync (return everything). |
| `page` | integer | 1-indexed page number |
| `page_size` | integer | Requested page size (50–500) |

**Response:**

```json
{
  "documents": [
    {
      "source_id": "KB0020524",
      "title": "2026 Company Holidays",
      "version": 1712150400,
      "last_modified": "2024-04-03T16:00:00Z"
    },
    {
      "source_id": "KB0019833",
      "title": "VPN Setup Guide",
      "version": 1711900000,
      "last_modified": "2024-03-31T10:00:00Z"
    }
  ],
  "has_more": true
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `documents` | array | yes | List of document stubs |
| `documents[].source_id` | string | yes | Unique identifier within this connector. Must be stable across syncs. |
| `documents[].title` | string | no | Document title (used for display; fetched content can override) |
| `documents[].version` | integer | no | Version number or epoch timestamp for change detection |
| `documents[].last_modified` | string | no | RFC 3339 timestamp of last modification |
| `has_more` | boolean | no | `true` if more pages exist. Default `false`. |

**Pagination contract:**

- DocBrain starts at `page=1` and increments until `has_more` is `false`
- DocBrain stops if it hits `CONNECTOR_MAX_PAGES_PER_SYNC` (default: 200) or `CONNECTOR_MAX_DOCS_PER_SYNC` (default: 5,000)
- The `since` parameter enables **incremental sync** — on subsequent syncs, DocBrain passes the `last_sync_at` timestamp so your connector only returns new or modified documents
- On the first sync, `since` is `null` — return all documents

---

### 3. Fetch Documents

```
POST {base_url}/documents/fetch
Authorization: {auth_header}
Content-Type: application/json
```

**Request:**

```json
{
  "source_ids": ["KB0020524", "KB0019833"]
}
```

**Response:**

```json
{
  "documents": [
    {
      "source_id": "KB0020524",
      "title": "2026 Company Holidays",
      "content": "# 2026 Company Holidays\n\nThe following dates are company-wide holidays for 2026:\n\n- **January 1** — New Year's Day\n- **January 19** — Martin Luther King Jr. Day\n...",
      "source_url": "https://docs.example.com/esc/en/hr-knowledge/2026-holidays",
      "metadata": {
        "category": "hr",
        "author": "HR Team",
        "tags": ["holidays", "benefits", "2026"]
      },
      "references": [
        {
          "url": "https://docs.example.com/esc/en/hr-knowledge/pto-policy",
          "title": "PTO Policy",
          "ref_type": "related"
        }
      ]
    }
  ]
}
```

| Field | Type | Required | Description |
|---|---|---|---|
| `documents` | array | yes | Full document objects |
| `documents[].source_id` | string | yes | Must match the ID from the list response |
| `documents[].title` | string | yes | Document title |
| `documents[].content` | string | yes | Document body. **Markdown preferred** — DocBrain's chunker is heading-aware. Plain text and HTML also work. |
| `documents[].source_url` | string | no | Link back to the original document. Shown in Q&A source citations. |
| `documents[].metadata` | object | no | Arbitrary key-value metadata. Stored alongside the document. |
| `documents[].references` | array | no | Cross-document references. Stored in the reference graph for enrichment at query time. |
| `documents[].references[].url` | string | yes | URL of the referenced document |
| `documents[].references[].title` | string | no | Title of the referenced document |
| `documents[].references[].ref_type` | string | no | Relationship type (e.g., `"related"`, `"linked"`, `"parent"`) |

**Batch contract:**

- DocBrain sends source IDs in batches of `CONNECTOR_FETCH_BATCH_SIZE` (default: 50)
- If a document cannot be fetched, omit it from the response — DocBrain will not treat missing documents as errors
- Maximum response body size: `CONNECTOR_MAX_RESPONSE_BYTES` (default: 10 MB)
- Request timeout: `CONNECTOR_REQUEST_TIMEOUT_SECS` (default: 30 seconds)

---

## Registering a Connector

### Via Web UI

1. Navigate to **Connectors** in the sidebar
2. Click **+ Register Connector**
3. Fill in the fields:

| Field | Example | Description |
|---|---|---|
| **Name** | `servicenow-kb` | Unique identifier (used in logs and API) |
| **Display Name** | `ServiceNow Knowledge Base` | Human-readable label shown in the UI |
| **Base URL** | `https://sn-connector.internal:8080` | Where your connector is running |
| **Source Type** | `servicenow` | Tag applied to all ingested documents (max 20 chars, must be unique) |
| **Auth Header** | `Bearer your-token` | Sent as the `Authorization` header on every request |
| **Cron Schedule** | `0 */6 * * *` | Standard 5-field cron expression |
| **Space** | `hr-knowledge` | Knowledge space assignment (optional — defaults to source type) |

4. Click **Register**

### Via API

```bash
curl -X POST https://your-docbrain/api/v1/connectors \
  -H "Authorization: Bearer <admin-api-key>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "servicenow-kb",
    "display_name": "ServiceNow Knowledge Base",
    "base_url": "https://sn-connector.internal:8080",
    "source_type": "servicenow",
    "auth_header": "Bearer your-token",
    "schedule_cron": "0 */6 * * *",
    "space": "hr-knowledge"
  }'
```

---

## Testing a Connector

### Health Check

Before triggering a full sync, verify connectivity:

```bash
# Via DocBrain API
curl -X POST https://your-docbrain/api/v1/connectors/{id}/test \
  -H "Authorization: Bearer <admin-api-key>"

# Or call your connector directly
curl https://sn-connector.internal:8080/health
```

### Manual Sync

Trigger a sync immediately without waiting for the cron schedule:

```bash
curl -X POST https://your-docbrain/api/v1/connectors/{id}/sync \
  -H "Authorization: Bearer <admin-api-key>"
```

Response:

```json
{
  "docs_synced": 42
}
```

### Verify Documents

After sync, confirm the documents are searchable:

```bash
# Check document count
curl "https://your-docbrain/api/v1/admin/documents?source_type=servicenow" \
  -H "Authorization: Bearer <admin-api-key>"

# Ask a question about the content
curl -X POST https://your-docbrain/api/v1/ask \
  -H "Authorization: Bearer <api-key>" \
  -H "Content-Type: application/json" \
  -d '{"question": "What are the 2026 company holidays?"}'
```

---

## Example: ServiceNow Connector (Python)

A minimal connector that exposes ServiceNow Knowledge Base articles to DocBrain:

```python
"""
ServiceNow Knowledge Base connector for DocBrain.

Usage:
    pip install flask requests
    SERVICENOW_INSTANCE=https://yourco.service-now.com \
    SERVICENOW_USER=api-user \
    SERVICENOW_PASS=api-pass \
    python servicenow_connector.py
"""
import os
from datetime import datetime
from flask import Flask, request, jsonify
import requests

app = Flask(__name__)

SN_INSTANCE = os.environ["SERVICENOW_INSTANCE"]
SN_AUTH = (os.environ["SERVICENOW_USER"], os.environ["SERVICENOW_PASS"])


@app.route("/health", methods=["GET"])
def health():
    return jsonify({"status": "ok", "connector_name": "servicenow-kb", "version": "1.0.0"})


@app.route("/documents/list", methods=["POST"])
def list_documents():
    body = request.json
    since = body.get("since")
    page = body.get("page", 1)
    page_size = body.get("page_size", 100)

    # Build ServiceNow Table API query
    query = "workflow_state=published"
    if since:
        # ServiceNow datetime format
        sn_since = since.replace("T", " ").replace("Z", "")
        query += f"^sys_updated_on>{sn_since}"

    offset = (page - 1) * page_size
    resp = requests.get(
        f"{SN_INSTANCE}/api/now/table/kb_knowledge",
        auth=SN_AUTH,
        params={
            "sysparm_query": query,
            "sysparm_fields": "sys_id,short_description,sys_updated_on",
            "sysparm_limit": page_size,
            "sysparm_offset": offset,
        },
    )
    resp.raise_for_status()
    records = resp.json()["result"]

    documents = []
    for r in records:
        documents.append({
            "source_id": r["sys_id"],
            "title": r["short_description"],
            "last_modified": r["sys_updated_on"].replace(" ", "T") + "Z",
        })

    return jsonify({
        "documents": documents,
        "has_more": len(records) == page_size,
    })


@app.route("/documents/fetch", methods=["POST"])
def fetch_documents():
    body = request.json
    source_ids = body.get("source_ids", [])

    documents = []
    for sid in source_ids:
        resp = requests.get(
            f"{SN_INSTANCE}/api/now/table/kb_knowledge/{sid}",
            auth=SN_AUTH,
            params={"sysparm_fields": "sys_id,short_description,text,sys_class_name"},
        )
        if resp.status_code != 200:
            continue  # skip missing articles
        r = resp.json()["result"]

        documents.append({
            "source_id": r["sys_id"],
            "title": r["short_description"],
            "content": r["text"],  # HTML — DocBrain handles conversion
            "source_url": f"{SN_INSTANCE}/kb_view.do?sys_kb_id={r['sys_id']}",
            "metadata": {"category": r.get("sys_class_name", "")},
        })

    return jsonify({"documents": documents})


if __name__ == "__main__":
    app.run(host="0.0.0.0", port=8080)
```

Deploy this alongside DocBrain (same cluster, sidecar, or any reachable endpoint), then register it:

```bash
curl -X POST https://your-docbrain/api/v1/connectors \
  -H "Authorization: Bearer <admin-key>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "servicenow-kb",
    "display_name": "ServiceNow Knowledge Base",
    "base_url": "http://servicenow-connector:8080",
    "source_type": "servicenow",
    "auth_header": "Bearer shared-secret",
    "schedule_cron": "0 */6 * * *",
    "space": "support"
  }'
```

---

## Example: Notion Connector (Node.js)

```javascript
/**
 * Notion connector for DocBrain.
 *
 * Usage:
 *   npm install express @notionhq/client
 *   NOTION_TOKEN=ntn_xxx node notion_connector.js
 */
const express = require("express");
const { Client } = require("@notionhq/client");

const app = express();
app.use(express.json());

const notion = new Client({ auth: process.env.NOTION_TOKEN });

app.get("/health", (_req, res) => {
  res.json({ status: "ok", connector_name: "notion", version: "1.0.0" });
});

app.post("/documents/list", async (req, res) => {
  const { since, page_size = 100 } = req.body;
  // Use Notion's search API with a last_edited_time filter
  const filter = since
    ? { property: "object", value: "page", timestamp: "last_edited_time", last_edited_time: { after: since } }
    : undefined;

  const response = await notion.search({
    filter: { property: "object", value: "page" },
    page_size: Math.min(page_size, 100),
    sort: { direction: "descending", timestamp: "last_edited_time" },
  });

  const documents = response.results
    .filter((p) => !since || p.last_edited_time > since)
    .map((p) => ({
      source_id: p.id,
      title: p.properties?.title?.title?.[0]?.plain_text || "Untitled",
      last_modified: p.last_edited_time,
    }));

  res.json({ documents, has_more: response.has_more });
});

app.post("/documents/fetch", async (req, res) => {
  const { source_ids = [] } = req.body;
  const documents = [];

  for (const id of source_ids) {
    try {
      const page = await notion.pages.retrieve({ page_id: id });
      const blocks = await notion.blocks.children.list({ block_id: id });
      const content = blocks.results
        .map((b) => {
          if (b.type === "paragraph") return b.paragraph?.rich_text?.map((t) => t.plain_text).join("") || "";
          if (b.type === "heading_1") return `# ${b.heading_1?.rich_text?.map((t) => t.plain_text).join("")}`;
          if (b.type === "heading_2") return `## ${b.heading_2?.rich_text?.map((t) => t.plain_text).join("")}`;
          if (b.type === "heading_3") return `### ${b.heading_3?.rich_text?.map((t) => t.plain_text).join("")}`;
          if (b.type === "bulleted_list_item") return `- ${b.bulleted_list_item?.rich_text?.map((t) => t.plain_text).join("")}`;
          if (b.type === "code") return `\`\`\`\n${b.code?.rich_text?.map((t) => t.plain_text).join("")}\n\`\`\``;
          return "";
        })
        .filter(Boolean)
        .join("\n\n");

      const title = page.properties?.title?.title?.[0]?.plain_text || "Untitled";
      documents.push({
        source_id: id,
        title,
        content,
        source_url: page.url,
      });
    } catch (err) {
      console.error(`Failed to fetch ${id}: ${err.message}`);
    }
  }

  res.json({ documents });
});

app.listen(8080, () => console.log("Notion connector listening on :8080"));
```

---

## Configuration Reference

These environment variables control connector behavior on the DocBrain server side:

| Variable | Default | Description |
|---|---|---|
| `CONNECTOR_ENABLED` | `true` | Enable/disable the connector scheduler globally |
| `CONNECTOR_MAX_CONCURRENT_SYNCS` | `3` | Maximum connectors syncing simultaneously |
| `CONNECTOR_MAX_PAGES_PER_SYNC` | `200` | Max pages to request from `/documents/list` per sync |
| `CONNECTOR_MAX_DOCS_PER_SYNC` | `5000` | Max documents to ingest per sync cycle |
| `CONNECTOR_FETCH_BATCH_SIZE` | `50` | Documents per `/documents/fetch` request |
| `CONNECTOR_REQUEST_TIMEOUT_SECS` | `30` | Timeout for each HTTP request to the connector |
| `CONNECTOR_SYNC_TIMEOUT_SECS` | `3600` | Overall timeout for a full sync cycle (1 hour) |
| `CONNECTOR_MAX_RESPONSE_BYTES` | `10485760` | Max response body size (10 MB) |
| `CONNECTOR_CIRCUIT_BREAKER_THRESHOLD` | `5` | Consecutive failures before auto-disabling the connector |
| `CONNECTOR_ALLOW_INTERNAL` | `false` | Allow `base_url` pointing to private/internal IPs (SSRF protection) |

---

## Error Handling and Circuit Breaker

DocBrain tracks connector health automatically:

- **Consecutive failures** are counted per connector. Each failed sync (health check failure, list failure, timeout) increments the counter. A successful sync resets it to zero.
- **Circuit breaker** — when `consecutive_failures` reaches the threshold (default: 5), DocBrain **auto-disables** the connector and emits a `ConnectorDisabled` event. This prevents a broken connector from wasting resources and flooding logs.
- **Re-enabling** — manually re-enable via the web UI toggle or API: `PATCH /api/v1/connectors/{id}` with `{"is_active": true}`.
- **Manual sync** bypasses the circuit breaker — useful for testing after fixing a connector.

### Monitoring

Check connector status in the web UI (Connectors page) or via API:

```bash
curl https://your-docbrain/api/v1/connectors/{id}/status \
  -H "Authorization: Bearer <api-key>"
```

Response:

```json
{
  "id": "...",
  "name": "servicenow-kb",
  "is_active": true,
  "last_sync_at": "2026-04-07T06:00:00Z",
  "last_sync_docs": 42,
  "last_error": null,
  "consecutive_failures": 0
}
```

### Events

| Event | When |
|---|---|
| `ConnectorSynced` | Successful sync — includes connector name and document count |
| `ConnectorDisabled` | Circuit breaker tripped — includes failure count and last error |
| `DocumentIngested` | Each document successfully ingested — includes title, space, chunk count |

Subscribe to these via the event bus (SSE) or outbound webhooks.

---

## Security Considerations

### SSRF Protection

By default, DocBrain rejects `base_url` values pointing to private/internal IP ranges (`10.x`, `172.16-31.x`, `192.168.x`, `127.x`, `localhost`). This prevents a malicious connector registration from probing your internal network.

If your connector runs inside the same cluster or private network, set:

```env
CONNECTOR_ALLOW_INTERNAL=true
```

### Authentication

The `auth_header` value is sent as-is in the `Authorization` header on every request. Use a shared secret, API key, or OAuth bearer token:

```
Bearer your-secret-token
Basic base64-encoded-credentials
ApiKey your-api-key
```

The auth header is stored in PostgreSQL in plaintext (consistent with how webhook secrets are stored). Use Kubernetes secrets or external secret managers to inject the value at registration time rather than hardcoding it.

### Network Isolation

For production deployments, run connectors in the same Kubernetes cluster or VPC as DocBrain. Use Kubernetes NetworkPolicies or security groups to restrict which pods can reach your connector.

---

## FAQ

**Q: Can I use a connector for a one-time import?**

Yes. Register the connector, trigger a manual sync (`POST /api/v1/connectors/{id}/sync`), then delete it or disable the cron schedule. The ingested documents remain in DocBrain.

**Q: What happens if my connector is down during a scheduled sync?**

The health check fails, the sync is skipped, and `consecutive_failures` increments by 1. After 5 consecutive failures (configurable), the connector is auto-disabled. No documents are lost — the next successful sync picks up from where it left off via the `since` parameter.

**Q: Can I have multiple connectors for the same source system?**

Yes, as long as each has a unique `name` and `source_type`. For example, `servicenow-hr` (source type: `sn_hr`) and `servicenow-it` (source type: `sn_it`) for different ServiceNow knowledge bases.

**Q: How does DocBrain handle document updates?**

DocBrain uses upsert logic keyed on `source_type` + `source_id`. When a document is re-fetched with updated content, the old chunks are deleted and new ones are indexed. The document retains its identity in PostgreSQL.

**Q: What format should `content` be in?**

**Markdown is preferred** — DocBrain's chunker splits at heading boundaries (`#`, `##`, `###`) for semantic coherence. Plain text works but produces less precise chunks. HTML is also accepted.

**Q: My source doesn't support incremental queries (no "modified since" filter). What do I do?**

Ignore the `since` parameter and return all documents every time. DocBrain's upsert logic handles duplicates — unchanged documents are re-indexed, which is slightly wasteful but correct. Consider implementing a version check in your connector to skip unchanged documents and return only their source IDs in the list response.

**Q: Can connectors delete documents from DocBrain?**

Not directly via the protocol. If a document is removed from the source, it will stop appearing in `/documents/list` responses and eventually become stale (scored by DocBrain's freshness system). To force-remove, use the admin API: `DELETE /api/v1/admin/documents/{source_type}/{source_id}`.
