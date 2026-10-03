# Access evidence sweeps: operator note

Two background jobs in the server keep the access evidence tables fresh: the
**membership sweep** (Confluence spaces and groups, Slack channels) and the
**document-state sweep** (Confluence pages, Jira issues). Each run is
all-or-nothing per scope, and a failed scope is stamped `failed` with a reason
code.

| Variable | Default | Range | Meaning |
|---|---|---|---|
| `ACCESS_MEMBERSHIP_SWEEP_SECS` | `240` | `60` to `480` | Seconds between membership sweeps. Membership evidence must stay usable for 10 minutes. |
| `ACCESS_STATE_SWEEP_SECS` | `900` | `300` to `1500` | Seconds between document-state sweeps. Document state must stay usable for 30 minutes. |

`0` disables a sweep. Unset or blank uses the default. Any other value outside
the range makes the server refuse to start, naming the variable and its range.

Sources are skipped when not configured:

- **Slack** needs `slack.bot_token` in the YAML configuration, or
  `SLACK_BOT_TOKEN`.
- **Jira** needs `sources.jira.base_url` and `sources.jira.api_token` (and
  `sources.jira.user_email` for Cloud basic auth), or `JIRA_BASE_URL`,
  `JIRA_API_TOKEN` and `JIRA_USER_EMAIL`. An environment variable wins over
  the YAML value.
- **Confluence** uses the server's Confluence client, which the server builds
  from the environment only: `CONFLUENCE_BASE_URL`, `CONFLUENCE_API_TOKEN` and
  `CONFLUENCE_USER_EMAIL`. A `confluence:` block in the YAML alone does not
  start the Confluence sweeps.

At startup the server logs one line, `access sweeps starting`, with both
intervals (`disabled` for `0`) and `yes` or `no` per source. A source that
reads `no` there is not swept: check its settings above.

## Things that will bite

- **Advisory locks.** Each sweep runs under a Postgres session advisory lock
  (`access.sweep.membership`, `access.sweep.state`), so only one replica runs a
  given sweep at a time. **PgBouncer in transaction mode breaks session
  advisory locks**: the lock and the unlock can land on different server
  connections. Point the server at Postgres directly, or a session-mode pool.
- **Data Center vs Cloud is decided by `CONFLUENCE_API_VERSION`** (default
  `v2`, Cloud). A Data Center install left on the default shows every
  Confluence group scope as `failed` / `http_403` on each tick, instead of
  `structurally_unavailable`. Set `CONFLUENCE_API_VERSION=v1` for Data Center.
- A run that outlasts its interval logs a warning and the missed ticks are
  skipped. A run that errors (for example the scope-listing query failed) is
  logged at `error` with the job name, and the next tick runs normally.
- **Jira Data Center.** The issue enumeration calls
  `/rest/api/3/search/jql`, which exists on Jira Cloud only. On Data Center
  every Jira project scope reads `failed` / `http_404`. This is a known
  limitation: Jira issue state is not captured on Data Center.

## Deploying

- **Order: server first, then ingest.** The server runs migration 202
  (`202_access_evidence.sql`) at startup. A new ingest image against a database
  that has not been migrated fails every document upsert (the upserts write
  `source_title`). An old ingest image running beside a new server can write
  real titles back over titles the sweep owns; the next sweep takes them
  again, so keep that window short.
- **The first ingest pass after the upgrade costs more.** Confluence bodies
  are now read in storage format, so the first ordinary ingest pass sees most
  Confluence pages as changed and re-chunks them. Expect one pass of embedding
  cost for the Confluence corpus, plus image-description cost where image
  descriptions are enabled. Later passes are back to normal.
- **Rolling back migration 202** gives every sweep-owned title its real title
  back before it drops the columns.

## Reading a failed scope

Each scope's latest outcome is one row in `access_scopes`. `status` is `read`,
`failed` or `unavailable`; `detail` is a reason code, never response text. A
failed scope keeps the members, rows and `as_of` of its last complete read, so
its evidence ages until a read succeeds.

Start here:

```sql
SELECT source, scope_kind, status, detail, count(*), max(now() - as_of) AS oldest
FROM access_scopes
GROUP BY 1,2,3,4
ORDER BY 1,2,3;
```

`docbrain admin access coverage` shows the same picture per source: `FAILED`
is the number of scopes whose latest read failed, `UNCAPTURED` the documents
without evidence fresh enough to decide from, `MEMBER P99` the age of the
oldest membership evidence (Confluence).

| Code | What it means | What to do |
|---|---|---|
| `http_401` | The source rejected the credentials (for Slack: an invalid, revoked or expired token). | Replace the token or API key for that source. |
| `http_403` | The credentials are valid but may not read this scope. For a Confluence group on Cloud: the site refused the group-membership call. For Slack's channel list: a missing scope or permission. | Grant the service account or bot the missing permission. On Confluence Data Center, set `CONFLUENCE_API_VERSION=v1` (see above). |
| `http_404` | The source does not know the endpoint the sweep called: usually a wrong base URL or context path, or Jira Data Center (see above). | Check the configured base URL, context path included. |
| `http_429` | Rate limited. Slack and Jira reads never wait on a rate limit; a Confluence read waits only when asked to wait 60 seconds or less. | Nothing if it clears on the next ticks. If it persists, lengthen the sweep interval or raise the source's rate limit. |
| `http_5xx` | The source answered with a server error after the sweep's retries. | Check the source's status; it clears when the source recovers. |
| `http_other` | Any other HTTP status. | Read the server log line for the scope and check the base URL and any proxy in between. |
| `transport` | No answer: DNS, TLS, connection or timeout. | Check network reachability from the server to the source. |
| `incomplete` | The answer could not be read completely: a field the sweep needs was missing, or a pagination link pointed outside the configured host. | Check that the configured base URL is the address the source itself uses (host and context path). If it persists, report it with the scope kind. |
| `empty_result` | The read succeeded but returned nothing while DocBrain holds documents for the scope. An empty answer is never taken as evidence. | Check that the service account can see the space or project at all. |
| `not_in_channel` | Slack: the bot is not a member of this channel. | Invite the bot to the channel. Until then its threads are not named after the channel: a thread that was reads `private Slack channel · date`. |
| `channel_not_found` | Slack: the channel is not in the bot's channel list (deleted, or private without the bot). | Same as `not_in_channel`. |
| `missing_scope` | Slack: the bot token lacks an OAuth scope the member list needs. | Add the scope (`channels:read` for public channels, `groups:read` for private ones) and reinstall the app. |
| `structurally_unavailable` | The deployment cannot ask at all: Confluence group membership on Data Center. Status is `unavailable`, not `failed`. | Nothing; it is a property of the deployment. |
| `cap_exceeded` | The listing did not end within the sweep's page cap (Confluence page sweep: 10,000 result pages; Confluence group: 100; Jira project: 2,000; Slack: 200). | Report it with the scope kind and its size; the caps are fixed in this release. |

## A sweep keeps skipping

Each tick that finds its lock held logs `access sweep skipped: another holder
has the lock` at `info`. With several replicas that is normal: one runs, the
others skip. From the third skip in a row the same job logs at `warn` and
names the job. That means no replica is finishing the job: either a holder is
stuck, or a transaction-mode pooler sits in front of Postgres and the locks
are landing on the wrong connections.

See who holds the locks:

```sql
SELECT pid, objid, granted FROM pg_locks WHERE locktype = 'advisory';
```

The two sweep locks have these `objid` values:

```sql
SELECT hashtext('access.sweep.membership')::bigint & 4294967295 AS membership,
       hashtext('access.sweep.state')::bigint & 4294967295      AS state;
```

Look the `pid` up in `pg_stat_activity` to see which client holds it and since
when. A holder that is a pooler connection, or one idle for longer than a
sweep interval, is the stuck one: closing that connection releases the lock
and the next tick runs.

## A title reads "Restricted Confluence page · SPACE · date"

The state sweep owns the title of every Confluence page it cannot show to be
readable by the whole space, and stores a neutral label instead. The real
title is kept and comes back by itself. A page gets the label when:

- the page has its own read restriction, or one of its ancestors has;
- an ancestor is not a page the sweep saw (a folder, or a page the service
  account cannot see), so the chain above the page cannot be checked;
- the page is archived: the sweep's listing does not return archived pages;
- Confluence's search index has not caught up with a new or moved page.

The label is released on the next state sweep that sees the page as readable
by the space (state `space`). Nothing needs to be done for index lag. For the
other cases the label is correct for as long as the cause holds.

The same rule applies to Jira: an issue the project listing no longer returns
(deleted, moved to another project, or hidden from the service account) reads
`Restricted Jira issue · PROJECT · date` until a sweep sees it again.
