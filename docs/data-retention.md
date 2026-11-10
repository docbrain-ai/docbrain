# Data Retention Policies

A retention policy is a rule about one source type: DocBrain holds nothing of that type whose last edit at the
source is older than the policy's age. The rule is kept at both ends:

- **A purge destroys what is too old.** The documents are deleted from the database and from the search index.
- **Ingest does not store what is too old.** An item the source still lists is not written while it is older than
  the age, so a purged item does not come back at the next ingest.

The source itself is never touched.

A policy is how old items leave for a source DocBrain reads through a lookback window (Jira, pull and merge
requests, Linear, Zendesk). Such an item is not removed when it drops out of the window; see `INGEST_PRUNE_DELETED`
in [Configuration](configuration.md).

---

## Which sources can have a policy

`jira`, `github_pr`, `gitlab_mr`, `linear`, `zendesk`

These record an edit date and are read through a lookback window, so a policy is the only way an old item leaves.
For any other type the API answers `400` and says why:

- **Confluence and local folders** list everything they hold on every ingest. A page leaves DocBrain when it
  leaves the source (`INGEST_PRUNE_DELETED`), and a purge there would be written back by the next ingest.
- **Slack threads, Microsoft Teams, PagerDuty, OpsGenie, Intercom, repository files, captures and custom
  connectors** record no edit date, so a policy by age would remove nothing.

Four things to know before you set one:

- **Age is the source's own edit date**, not the day DocBrain ingested the item. For a Jira issue it is the issue's
  last update. An item that is still being edited never ages out, and an old item that is edited again is stored
  again at the next ingest that lists it.
- **A document with no recorded edit date is never removed by a policy.** Every purge reports how many it kept for
  that reason (`undated_kept`).
- **A purge cannot be undone from DocBrain.** The rows are deleted, not marked. Raising the age or switching the
  policy off does not bring anything back by itself: an item returns only when a later ingest lists it again, which
  for these sources means it is inside the lookback window (for Jira, `JIRA_LOOKBACK_DAYS`).
- **An age shorter than the lookback window is allowed.** Ingest then reads items it does not store; the ingest log
  counts them under *Skipped* as "older than the retention policy's age, not written".

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
| `max_age_days` | integer from 1 to 36500, or null | null | Hold nothing whose last edit at the source is older than this many days. `null` keeps everything |
| `enabled` | boolean | true | Whether the policy is in force. Switched off, nothing of this type is purged and ingest stores items of any age |

A policy is **in force** when it is enabled and has an age. Setting one does not purge anything at that moment:
the first scheduled purge comes a day later (see below), and ingest stops storing older items from its next run.

`purge_vectors` is no longer a setting. A purge always removes a document from the search index together with the
document. A request that still sends `"purge_vectors": true` is accepted; `"purge_vectors": false` is answered
`400`.

### Purge now

```bash
curl -X POST \
  -H "Authorization: Bearer $ADMIN_KEY" \
  https://docbrain.mycompany.com/api/v1/admin/retention/jira/purge
```

Destroys at once what the policy in force says is too old, and answers:

```json
{ "source_type": "jira", "docs_purged": 24, "chunks_purged": 480, "cutoff_date": "2025-10-06T09:12:44Z", "undated_kept": 3, "recently_written_kept": 0 }
```

| Answer | Meaning |
|---|---|
| `200` | The purge reached its end. `cutoff_date` is the instant it purged up to. `undated_kept` is the number of documents of this type that have no edit date and were kept. `recently_written_kept` is the number past the age that an ingest wrote within the last hour: the next purge takes them |
| `409` | Nothing was purged, because no policy is in force for this source type: none is set, it is switched off, it has no age, or the type cannot have a policy. The body says which |
| `500` | Either the purge stopped part-way, and the body says how many documents it removed first (the rest are kept; purge again once the cause is fixed), or an internal error |

The purge keeps running if the request is cut off (a closed terminal, a proxy timeout). Its result is in the audit
log below.

---

## Scheduled purge

Each policy in force is purged once a day:

- the first time, a day after the policy was last changed (its age, or switched on);
- then a day after the start of its last scheduled purge.

The schedule is read from the database, so a server restart does not move it: the server asks once a minute what
is due. Setting the same policy again does not restart the day. "Purge now" does not move the schedule either.

How a purge works, scheduled or manual:

1. It takes the documents of the source type whose last edit at the source is older than the age, a hundred at a
   time. A document an ingest wrote within the last hour is left for the next purge: the ingest may still be
   adding it to the search index.
2. Each document is removed from the search index first, then deleted from Postgres with its chunks and
   everything attached to it. Every answer that drew on it loses its text and the sources it quoted, a turn the
   asker left after the answer was shown included, so the answer cache cannot serve it again and its answer page
   says the answer was erased.
3. If the search index cannot remove a document, that document is kept and the purge stops there. The audit log
   says why, and the next purge starts again from it. A document is never deleted while the search index still
   holds it, because it would go on answering questions.

What a purge does not reach: a conversation in progress keeps its earlier turns until 30 minutes after its last
question, so a later answer in it can repeat purged text; an answer that was being written while the purge ran is
stored after it; the short description of a named thing (a service, a team) and the one-line resolution of an
incident that DocBrain drew from an answer stay until a later answer replaces them; and a later answer that used
an erased answer as background keeps its text, because DocBrain records which documents an answer was drawn from,
not which earlier answers it leaned on. A list of document titles that the asker left in the moment between its
text and the end of the answer keeps those titles. Rows recorded before the release that brought this (#765 in
the changelog) are reached only in part: an answer made from a document an earlier purge removed keeps its text,
since nothing recorded which documents those purges removed; a turn the asker left after the answer was shown
recorded no documents, so no purge finds it, and it stays until the episode itself is pruned; and a turn stopped
at the time limit after text was shown says "stopped" on its answer page, not "erased", once a purge erases it.

---

## Audit log

Every purge, scheduled or manual, writes one row when it starts. When it ends, the row gets the counts and, if the
purge stopped early, the reason:

```sql
SELECT source_type, trigger, triggered_by, docs_purged, chunks_purged, cutoff_date, started_at, finished_at, error_message
FROM retention_purge_log
ORDER BY started_at DESC
LIMIT 20;
```

`trigger` is `scheduler` or `api`; `triggered_by` is `system` or the admin's user id. `docs_purged` is what that
purge removed, also when `error_message` says it stopped early. A row with no `finished_at` is a purge that was
cut short by the server stopping, or one that finished but could not close its row, which the server log then
says.

---

## Common policies

```bash
# Hold no Jira issue that has not been updated for 400 days
curl -X PUT -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 400}' \
  .../api/v1/admin/retention/jira

# Hold no Zendesk ticket that has not been updated for a year
curl -X PUT -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 365}' \
  .../api/v1/admin/retention/zendesk

# Keep the policy but take it out of force: nothing is purged, and ingest stores items of any age again
curl -X PUT -H "Authorization: Bearer $ADMIN_KEY" \
  -H "Content-Type: application/json" \
  -d '{"max_age_days": 400, "enabled": false}' \
  .../api/v1/admin/retention/jira
```

---

## Upgrading from 1.6.5 or earlier

Until 1.6.5 the scheduled purge ran only on a server that had been up for a full day, ingest stored a purged item
again when the source still listed it, and a policy could be set for Confluence and local folders, where it could
not hold. The upgrade changes three things by itself:

- A policy stored for `confluence` or `local` is switched off. It stays listed, so you can see what was set.
- Every policy in force gets a day from the moment of the upgrade before its first scheduled purge. If a policy
  has been set for a long time on a server that restarted often, this may be the first purge it ever gets: check
  the age in the list before that day is over.
- "Purge now" on a policy that is switched off, which used to purge with the stored age, is answered `409`.

---

## Database tables

**`source_retention_policies`**: one row per source type that has a policy. `updated_at` is when the age or the
switch last changed.
```sql
SELECT source_type, max_age_days, enabled, updated_at
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
