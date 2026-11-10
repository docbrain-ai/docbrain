# Data Retention Policies

A retention policy removes DocBrain's copy of the old items of one source type. Every document of that type whose
last edit at the source is older than the policy's age is deleted from the database and from the search index. The
source itself is never touched.

A policy is how old items leave for a source DocBrain reads through a lookback window (Jira, pull and merge
requests, Linear, Zendesk). Such an item is not removed when it drops out of the window; see `INGEST_PRUNE_DELETED`
in [Configuration](configuration.md).

---

## Which sources can have a policy

A policy can be set for the source types that record an edit date:

`confluence`, `jira`, `github_pr`, `gitlab_mr`, `linear`, `zendesk`, `local`

For any other type (Slack threads, Microsoft Teams, PagerDuty, OpsGenie, Intercom, repository files, captures,
custom connectors) the API answers `400` and says why: those sources record no edit date, so a policy by age would
remove nothing.

Three things to know before you set one:

- **Age is the source's own edit date**, not the day DocBrain ingested the item. For a Jira issue it is the issue's
  last update. An item that is still being edited never ages out, and a document with no recorded edit date is
  never removed by a policy.
- **A purge deletes DocBrain's copy only.** If the source lists the item again on a later ingest run, it is
  ingested again. For a lookback source, set an age at least as long as its lookback window (for Jira,
  `JIRA_LOOKBACK_DAYS`). Confluence and local folders list everything on every run, so a purged page returns on the
  next run for as long as it exists at the source.
- **A purge cannot be undone from DocBrain.** The rows are deleted, not marked. Getting an item back means
  ingesting it again from the source.

---

## Admin API

All three calls need an admin key.

### List all policies

```bash
curl -H "Authorization: Bearer $ADMIN_KEY" \
  https://docbrain.mycompany.com/api/v1/admin/retention
```

### Set a policy

```bash
curl -X PUT \
  -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 400, "enabled": true}' \
  https://docbrain.mycompany.com/api/v1/admin/retention/jira
```

| Field | Type | Default | Description |
|---|---|---|---|
| `max_age_days` | integer (1 or more) or null | null | Remove items whose last edit at the source is older than this many days. `null` keeps everything |
| `enabled` | boolean | true | Whether the scheduled pass applies this policy. A manual purge uses the policy's age even when this is `false` |
| `purge_vectors` | boolean | true | Stored, not used: a purge always removes the item from the search index as well |

### Purge now

```bash
curl -X POST \
  -H "Authorization: Bearer $ADMIN_KEY" \
  https://docbrain.mycompany.com/api/v1/admin/retention/jira/purge
```

Applies the policy's age to that source type at once and answers with `docs_purged`, `chunks_purged` and
`cutoff_date`. The source type needs a policy with an age first; without one the call fails.

---

## Scheduled Purge

The server applies every enabled policy that has an age once every 24 hours, counted from the moment the server
started. The first pass is 24 hours after start, and a restart begins the count again, so a server that restarts
more often than once a day does not reach a pass. On such a deployment, call "Purge now" from your own scheduler.

Each pass, for each policy:
1. Finds the documents of that source type whose last edit at the source is older than `max_age_days`
2. Removes them from the search index
3. Deletes them from Postgres (documents, and their chunks by cascade)
4. Records the purge in `retention_purge_log`

---

## Audit Log

Every purge, scheduled or manual, writes one row when it starts and fills in the counts when it ends:

```sql
SELECT source_type, trigger, triggered_by, docs_purged, chunks_purged, cutoff_date, started_at, finished_at
FROM retention_purge_log
ORDER BY started_at DESC
LIMIT 20;
```

`trigger` is `scheduler` or `api`; `triggered_by` is `system` or the admin's user id. A row with no `finished_at`
is a purge that did not finish.

---

## Common Policies

```bash
# Remove Jira issues not updated for 400 days
curl -X PUT -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 400}' \
  .../api/v1/admin/retention/jira

# Remove Zendesk tickets not updated for a year
curl -X PUT -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 365}' \
  .../api/v1/admin/retention/zendesk

# Keep a policy but stop the scheduled pass from applying it
curl -X PUT -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 400, "enabled": false}' \
  .../api/v1/admin/retention/jira
```

---

## Database Tables

**`source_retention_policies`**: one row per source type that has a policy.
```sql
SELECT source_type, max_age_days, purge_vectors, enabled, updated_at
FROM source_retention_policies;
```

**`retention_purge_log`**: one row per purge.
```sql
SELECT * FROM retention_purge_log ORDER BY started_at DESC;
```

## Answer pages and answer topics

An answer's page (`/answers/<id>`) lives exactly as long as its answer: it stops opening once the answer is
older than `EPISODE_RETENTION_DAYS` (default 90) and says "Kept until <date>" while it is open. There is no
separate clock. Answers that used a capture are grouped into answer topics so the capture's author can see
what they were about; a topic holds a short label and a centroid, never a question or who asked. A pruned
answer leaves its topic, and topics with no answers left are removed hourly.
