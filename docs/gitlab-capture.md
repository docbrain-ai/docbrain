# GitLab MR Capture

GitLab MR Capture lets DocBrain learn directly from your engineering work in real time. When a team member comments `@docbrain capture` on a merge request, DocBrain ingests the MR title, description, and full discussion thread — turning institutional knowledge that lives in GitLab into searchable, citable documentation.

Beyond capture, DocBrain can also **answer questions** inline: comment `@docbrain <question>` on any MR and DocBrain will reply with a RAG-generated answer sourced from your knowledge base.

---

## How It Works

```
1. Team member comments "@docbrain capture" on a GitLab MR
          │
          ▼
2. GitLab fires a webhook → POST /api/v1/gitlab/events
          │
          ▼
3. DocBrain verifies X-Gitlab-Token, checks user/project allowlists
          │
          ▼
4. Fetches MR notes via GitLab API, builds structured document:
     Title:   "GitLab MR !42: Fix OAuth redirect (group/project)"
     Content: description + thread of human comments
     URL:     https://gitlab.com/group/project/-/merge_requests/42
          │
          ▼
5. Embeds + indexes all content (same pipeline as Confluence/Slack)
          │
          ▼
6. Posts ✅ confirmation note on the MR with chunk count
          │
          ▼
7. Content immediately available in Q&A and Autopilot gap analysis
```

### What Gets Captured

- MR title
- MR description (markdown)
- All human comment notes (system notes are excluded)
- **Cross-document references** — URLs in the description and notes are automatically extracted and classified (GitHub PRs, Jira tickets, Confluence pages, other GitLab MRs/issues, etc.). GitLab shorthand references (`!123` for MRs, `#123` for issues) are resolved to full URLs within the same project. Referenced documents are linked in a reference graph and enriched at query time.
- Stored as `source_type = "gitlab_capture"` with metadata: `project`, `mr_iid`, `mr_state`, `type: "merge_request"`

### Content Limits

- Minimum 200 characters — very short MRs are silently skipped
- Maximum 500 KB — oversized MRs are skipped with a warning log

---

## Setup

### Step 1 — Set Environment Variables

```bash
# Required: at least one of these must be set
GITLAB_CAPTURE_WEBHOOK_SECRET=<your-webhook-secret>   # validates X-Gitlab-Token header
GITLAB_CAPTURE_TOKEN=<gitlab-pat>                      # PAT with api scope (for fetching MR notes)

# Optional
GITLAB_CAPTURE_BASE_URL=https://gitlab.com             # default; change for self-hosted GitLab
GITLAB_CAPTURE_ALLOWED_USERS=alice,bob                 # only process commands from these usernames
GITLAB_CAPTURE_ALLOWED_PROJECTS=group/repo,org/app     # only process these projects
```

**`GITLAB_CAPTURE_TOKEN`** (PAT) is needed to fetch the full MR note thread via the GitLab API. Without it, DocBrain can only read the triggering note body, not the full discussion.

Required PAT scopes: **`api`** (read MR notes and post replies).

### Step 2 — Configure the GitLab Webhook

1. Go to your GitLab project (or group for org-wide capture) → **Settings → Webhooks**
2. **URL:**
   ```
   https://<your-docbrain-host>/api/v1/gitlab/events
   ```
3. **Secret token:** value of `GITLAB_CAPTURE_WEBHOOK_SECRET`
4. **Trigger:** check **Note events** (comments on MRs)
5. **SSL verification:** enabled (recommended)
6. Click **Add webhook**

**Test it:** click **Test → Note events** in the webhook list. DocBrain will respond 200 (the test payload has no `@docbrain` mention so nothing gets captured, but you'll confirm connectivity).

### Step 3 — Restart DocBrain

DocBrain reads the env vars at startup:

```
INFO GitLab capture webhook enabled — listening on POST /api/v1/gitlab/events
```

If you see `NOT_FOUND` on the webhook, `GITLAB_CAPTURE_WEBHOOK_SECRET` or `GITLAB_CAPTURE_TOKEN` is not set.

---

## Using Capture

### Capture an MR

Comment on any open or merged MR:

```
@docbrain capture
```

DocBrain replies within seconds:

```
✅ Captured by DocBrain — 7 chunks indexed and immediately searchable.
This MR will feed Autopilot's next gap analysis run.
```

The MR content is now searchable via the Q&A API and the web UI.

### Ask a Question on an MR

```
@docbrain How does the new auth middleware handle token refresh?
```

DocBrain replies with a sourced answer:

```
DocBrain — answering: How does the new auth middleware handle token refresh?

The middleware checks the token expiry window (configurable via TOKEN_REFRESH_WINDOW_SECS)...

Sources: [Auth Architecture](https://...) · [Confluence: Auth Design] · Confidence: 87%
```

### Commands Summary

| Comment | Effect |
|---|---|
| `@docbrain capture` | Index the full MR thread into DocBrain |
| `@docbrain <question>` | Run RAG on your knowledge base and post the answer |

Both triggers are **case-insensitive**. The `@docbrain` prefix must appear somewhere in the note.

---

## Access Control

### User Allowlist

By default, any commenter can trigger capture. To restrict to specific users:

```bash
GITLAB_CAPTURE_ALLOWED_USERS=alice,bob,ci-bot
```

Anyone not in this list who comments `@docbrain capture` will be silently ignored (DocBrain logs a `INFO` message, no error reply to GitLab).

### Project Allowlist

To limit capture to specific projects:

```bash
GITLAB_CAPTURE_ALLOWED_PROJECTS=engineering/backend,data/pipelines
```

Use `group/project` format as shown in the GitLab URL. Case-insensitive.

Both allowlists can be combined — the request must pass both checks.

---

## Self-Hosted GitLab

Set `GITLAB_CAPTURE_BASE_URL` to your instance URL:

```bash
GITLAB_CAPTURE_BASE_URL=https://gitlab.internal
```

If your instance uses a self-signed certificate, also set:

```bash
GITLAB_TLS_VERIFY=false
```

> ⚠️ Disabling TLS verification exposes the GitLab API token to interception. Use only in isolated networks.

---

## Batch Ingest (Without Webhooks)

To backfill existing MRs without waiting for new comments, use the batch ingest source:

```bash
# .env or environment
GITLAB_URL=https://gitlab.com
GITLAB_TOKEN=<pat-with-api-scope>
GITLAB_GROUPS=engineering,platform          # comma-separated group names
GITLAB_MR_STATE=merged                      # open | merged | closed | all
GITLAB_MR_LOOKBACK_DAYS=90                  # how far back to scan
```

Then trigger a manual ingest:

```bash
docbrain ingest                   # CLI
# or
POST /api/v1/admin/ingest         # API (admin key required)
```

Batch-ingested MRs are stored as `source_type = "gitlab_mr"` (distinct from `"gitlab_capture"` for webhook-triggered capture).

---

## Helm Chart Configuration

```yaml
# values.yaml
gitlab:
  # Webhook-triggered real-time capture
  captureEnabled: true
  captureBaseUrl: "https://gitlab.com"        # or your self-hosted URL
  captureAllowedUsers: ""                     # comma-separated; empty = allow all
  captureAllowedProjects: ""                  # comma-separated; empty = allow all

  # Secrets — set via external secret or --set
  # captureWebhookSecret: ""                  # GITLAB_CAPTURE_WEBHOOK_SECRET
  # captureToken: ""                          # GITLAB_CAPTURE_TOKEN
```

Secrets are typically managed via `kubectl create secret` or an external secrets operator:

```bash
kubectl create secret generic docbrain-gitlab \
  --from-literal=GITLAB_CAPTURE_WEBHOOK_SECRET=<secret> \
  --from-literal=GITLAB_CAPTURE_TOKEN=<pat>
```

Reference in your `values.yaml`:

```yaml
envFrom:
  - secretRef:
      name: docbrain-gitlab
```

---

## Monitoring

### Logs to Watch

| Log message | Meaning |
|---|---|
| `GitLab capture webhook enabled` | Startup: capture is active |
| `GitLab capture: MR !N in group/project — X chunks indexed` | Successful capture |
| `GitLab webhook: invalid or missing X-Gitlab-Token` | Webhook secret mismatch |
| `GitLab capture: MR !N in group/project exceeds 500KB` | MR too large, skipped |
| `GitLab capture: user 'X' not in GITLAB_CAPTURE_ALLOWED_USERS` | Access control filtered |

### Verify a Captured MR

```bash
# Check the document exists
GET /api/v1/admin/documents?source_type=gitlab_capture

# Or search
POST /api/v1/ask
{ "question": "content from the MR you captured" }
```

---

## FAQ

**Q: DocBrain replied with ✅ but I can't find the content in search.**

The content is indexed immediately but embedding propagation to OpenSearch can take a few seconds. Wait 5–10 seconds and retry. If it's still missing, check the `docbrain-server` logs for embedding errors.

**Q: No reply was posted on the MR at all.**

`GITLAB_CAPTURE_TOKEN` is likely not set. Without a PAT, DocBrain cannot call the GitLab API to post notes. Check the logs for `GitLab capture: ingest failed` or `not in GITLAB_CAPTURE_ALLOWED_USERS`.

**Q: Can I capture MRs across an entire group (all projects)?**

Yes — configure the webhook at the **Group** level in GitLab: **Group → Settings → Webhooks**. Use the same URL and secret. Leave `GITLAB_CAPTURE_ALLOWED_PROJECTS` empty to accept from all projects in the group.

**Q: Does capture work on closed/merged MRs?**

Yes. The `@docbrain capture` trigger works regardless of MR state. The MR state is stored in metadata and available in search results.

**Q: How is this different from the GitLab batch ingest?**

| | Webhook capture | Batch ingest |
|---|---|---|
| Trigger | `@docbrain capture` comment | Scheduled or manual |
| Latency | Real-time (seconds) | Next ingest cycle |
| Scope | One MR at a time | All MRs matching filters |
| Source type | `gitlab_capture` | `gitlab_mr` |
| Requires PAT | Yes (for notes API) | Yes |
| Requires webhook | Yes | No |

Use batch ingest to backfill history; use webhook capture for ongoing real-time knowledge capture.
