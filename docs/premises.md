# Premise Monitors — Living Claims

Every knowledge base stores claims whose truth quietly depends on facts elsewhere: a runbook
names a script, a decision record cites a config file, an onboarding doc points at a directory.
When the fact moves or disappears, the claim stays behind — confident, grounded, and wrong.

DocBrain attaches the checkable fact to the claim at capture time and keeps checking it.
A **premise** is a machine-verifiable condition a knowledge fragment asserts — in v1, a file
path in one of your connected git sources. A standing monitor re-verifies every premise on a
cycle, and a claim whose premise died tells you so, with evidence.

The whole pipeline is deterministic: no LLM call, no extra network request in any check.
A premise check is a set lookup against the file listing your git ingest already records.
It behaves identically on a laptop and in production.

## In plain terms

Every guide and runbook quietly depends on real things — "run this script", "edit that
file". When those things move or get deleted, the document doesn't know. It keeps giving
the old answer, confidently.

DocBrain writes down, at the moment knowledge is saved, exactly which real things it
depends on. Then a watchman checks that list against your actual repositories, every five
minutes, forever. The moment a fact your knowledge relies on disappears, that specific
piece of knowledge is flagged — with proof: *what* vanished, *when*, and where it went if
that can be determined. Your team gets a message; nobody has to reread anything.

And it never cries wolf: it only watches facts it has personally confirmed were true, it
never guesses between possibilities, and when it can't check something it says "can't
check" — never "broken".

## Where premises come from

1. **Automatic extraction.** When a fragment is indexed, path-shaped tokens in inline code
   spans (a filename with an extension, e.g. `` `scripts/rotate.sh` ``) are extracted as
   premises. Prose mentions, globs, URLs, flags, and placeholders are ignored — the same
   grammar the [answer-time claim verifier](configuration.md) uses.
2. **Explicit declaration.** Agents capturing over MCP can pass a `premises` array to
   `docbrain_annotate` or `docbrain_commit_capture` — see [Agent Capture](agents.md). v1
   checks `premise_type: "path"`; other types are recorded but never checked (and never
   alert). Malformed entries are skipped item-by-item — a bad premise never fails the
   capture. The array is capped at **100 items**; a capture declaring more is rejected with
   `400` rather than truncated, because the sweep reads every premise row in one unpaged
   query and that cap is what keeps its working set bounded by fragment volume.
3. **A commit capture's changed files.** `docbrain_commit_capture` records why a change was
   made; the server turns each path in its `file_paths` list into a `path` premise, so the
   capture is checked against the files it describes and is flagged when one of them moves
   or disappears. Without it a commit capture could never be checked against the code again.
   Repeats collapse to one premise, and the same 100-item cap applies. A commit capture sent
   with no files declares nothing; it is served at once, labelled *Not checked against code*.
4. **Backfill.** `POST /api/v1/premises/backfill` (admin) extracts premises for every
   already-indexed fragment that has none. Safe to re-run; it never duplicates.

## The four states

| State | Meaning | Alerts? |
|---|---|---|
| `holds` | Verified true against a connected source's listing | — |
| `broken` | Was verified true, now decisively absent | **yes** — `premise.broken` |
| `uncheckable` | No connected source can currently speak to it | never |
| `dormant` | Never verified true | never |

### How a reader sees a premise

The state is the monitor's; what it means to a reader also depends on the last verdict and on
whether anything has checked the premise yet. The server derives one reading and sends it beside the
raw `state` as `reading` (`state`, `label`, `checked_at`) on the capture page
(`GET /api/v1/fragments/{id}/page`) and the premise lists (`GET /api/v1/premises/broken`). Every
surface shows `label` as it comes and never labels a premise from `state` alone.

| `reading.state` | `label` | When |
|---|---|---|
| `holds` | Holds | `holds` |
| `broken` | Broken | `broken` |
| `not_found` | Not found in any connected repository | a `dormant` path premise whose last check found it in no listing (`missing` or `moved`) |
| `not_checked_yet` | Not checked yet | a `dormant` premise nothing has checked yet |
| `does_not_match` | Does not match the connected repository | a `dormant` version or config-key premise whose value, at the last check, differs from the repository's |
| `cannot_be_checked` | Cannot be checked now · last verified <date> | `uncheckable`, last verdict `verified` |
| `cannot_be_checked` | Cannot be checked now · last found missing <date> | `uncheckable`, last verdict `missing` (or `moved`, for a path) |
| `cannot_be_checked` | Cannot be checked | `uncheckable` with nothing kept to say, or a `dormant` premise whose last check found no connected source that owns it |

The `<date>` (UTC, `YYYY-MM-DD`) is the day the premise stopped being checkable: its last verdict was
current up to then.

`checked_at` is when a check last ran, and `null` while nothing has checked the premise: a new row's
`last_checked_at` is its creation time, not a check. A `not_found` premise is still dormant: it never
alerts and never appears on an answer.

The sweep reads every path a capture declares, including one whose top folder no connected
repository holds: such a path reads *Not found in any connected repository* with its check time, and
turns to *Holds* when a repository gains it.

**A reader is told only what the repositories they can read say.** A repository's listing is
readable when the reader can read every page DocBrain holds from it, by the rule Ask applies (spaces
and access rules; a listing with no page yet is readable unless an enforcing access rule applies).
For a reader who cannot read every listing, a claim whose path (or, for a version or config-key
claim, whose file) no readable listing holds, and whose recorded holder they cannot read, reads *Not
found in any repository you can read* where it would otherwise say it holds, broke, was last
verified, or that its value differs; its raw `state` reads `dormant` and its `state_changed_at` the
time the claim was written. The capture's chip, the Captures tiles, the "code changed" lists and a
capture's own list of broken claims follow the same readings: every capture response row with
premise data is built from the reader's scope. Which listings a reader can read is worked out once
per kind of reader and kept until the listings next refresh, at most one refresh interval.

**The capture chip** comes from the readings too: *Checked against code* only when every claim
holds; *Code it rests on changed* when a claim broke; *A claim does not hold* when any other claim
does not hold (one not checked yet beside ones that were included); *Not checked against code* when
nothing has been checked.

Summaries count readings, not states. The Captures tiles say "all still match the code" only when
every claim of every live capture in view reads *Holds*; when nothing has broken but some claims do
not hold, the tile reads "<n> rest on a claim that does not hold" and links to those captures. The
Mine impact (`GET /api/v1/captures/mine/impact`) and Organisation (`GET /api/v1/captures/organisation`)
responses carry `not_holding` (live captures with any claim that does not read *Holds*) and
`not_holding_captures` (them, by summary; at most 10 on Organisation) beside `code_changed`
(captures with a broken claim). A claim found missing at its first check never breaks, so
`code_changed` alone cannot tell.

Home's *Wrong now* line reads its claims the same way: "N were checked and not found" for document
claims checked and found missing, "N point at files that moved" for those found at another path, and
"have not been checked" only for the claims nothing has checked. Its coverage records `not_found` and
`moved` inside `unchecked` (the complement of the base, which the review wall still states).

Two rules keep this honest:

- **Only a claim proven true can die.** A premise that fails verification at capture is
  born `dormant` and stays silent forever — so a placeholder path in an example can never
  page anyone. If a later check finds a dormant premise true, it is promoted to `holds`
  (silently) and monitored from then on.
- **"I can't check" is not "it's gone."** If a premise's source is disconnected or its
  listing pruned, the premise becomes `uncheckable` — an infrastructure fact, not an alarm.
  It keeps its last verdict and basis, so you can see what was known and when. When the
  source returns, the next sweep re-verdicts it; only a genuine death alerts.

**A capture that is false about its code at birth is refused.** A person's capture whose declared
`path` premise names a file that a connected repository was recorded holding (a premise's
`source_ref`) and that no connected repository holds now is not saved: the write answers `422`
`premise_removed`, naming every such path and never the repository. A file the author is adding has
no such record, so it is never refused; it is served with a warning.

**Every declared path that no listing the writer can read holds is warned about at save time**, on
a create and on an edit that sends premises (`warnings` in the response; `docbrain_annotate`,
`docbrain_commit_capture`, `docbrain_revise` and the console show them). Only listings the writer can
read are consulted: a repository they cannot read gives no branch, and no silence either. The warning
names the branch DocBrain read, never the repository:

- "the file this capture rests on does not exist on branch <branch>: <path>" when a connected
  listing owns the path's folders (the first two, three under a `.`-led top folder; with several, the
  smallest source key). The branch is the git source's tracked branch, or the branch checked out in
  a local source's working copy at its last refresh; a branch DocBrain could not read, or a detached
  HEAD, is never guessed.
- "the file this capture rests on does not exist: <path>" when that listing's branch is not known (a
  detached HEAD, or a listing stored before branches were recorded).
- "the file this capture rests on is not in any repository you can read: <path>" when no listing
  the writer can read owns its folders.

It is a warning, not a refusal: the capture is saved. With no connected listing loaded, nothing
warns.
A commit's own file list, the premises an edit keeps, and DocBrain-written summaries are never
refused; with no connected listing loaded, nothing is.

## The monitor

With `PREMISE_MONITOR_ENABLED=true` (the default), the server runs a standing monitor:

- **Every 300 seconds by default** (`PREMISE_SWEEP_INTERVAL_SECS`) it sweeps all premises against the current file listings.
- **On fragment-index events** it extracts and initially verifies a new fragment's premises
  immediately, rather than waiting for the sweep.
- **At startup, and after any missed events,** it backfills premises for indexed fragments
  that have none, so nothing is silently unmonitored.
- **A discarded capture's premises are not swept.** Discarding a capture rejects it, so its
  claims stop being checked and stop being reported: they do not change state again, and
  `POST /api/v1/premises/check` never names a discarded capture. The rows are kept, frozen
  with the state and verdict they held at the moment of the discard — the same choice the
  discard itself makes, where the search entry goes and the capture stays with its reason —
  so the record of what was claimed survives the rejection. A capture that is merely waiting
  for review is not affected; only a rejected one stops being checked.

Writes happen only when something changed; a sweep over an unchanged world writes nothing
and emits nothing. If the monitor cannot check (database hiccup, missing listing), premises
hold their last-known state and age visibly — the system can be late and says so; it cannot
be confidently wrong because checking was broken.

`GET /api/v1/premises/health` reports the last sweep time and counts. The Premises page in
the web UI shows a visible warning when the monitor has not swept recently.

## Verdicts and evidence

Every decisive check records a verdict and a dated basis:

- `verified` — the path exists (recorded with its resolved location).
- `moved` — the path is gone, but same-named files exist in the owning source. With exactly
  one candidate the verdict names it; with several it reports the count and **refuses to
  pick** — a confident guess is the failure this feature exists to remove.
- `missing` — gone, and no same-named file exists. Deleted, not moved.

The basis line — `github:owner/repo at <commit> (captured <date>)` — states what the check
was evidence *of*. A listing is a snapshot, never a timeless truth; an absence is always
dated to the commit it was observed at.

## Surfaces

- **Web:** the **Premises** page — broken premises newest-first with verdicts and bases, a
  collapsed Uncheckable section, and monitor health.
- **API:** `GET /api/v1/premises/broken` (any state via `?state=`, paginated, analyst+),
  `GET /api/v1/premises/health` (analyst+), `POST /api/v1/premises/backfill` (admin),
  `POST /api/v1/premises/check` (viewer; the pull-request check below).
- **Pull requests:** `docbrain check-claims` — see [The pull-request check](#the-pull-request-check).
- **What is counted:** a claim made by a document that has since been deleted is not current
  content. The page, `/premises/health` and the pull-request check leave it out. The sweep
  still reads such rows, so beside its figure the page says how many of the rows it reads
  belong to deleted documents.
- **Events:** `premise.broken` and `premise.restored` are emitted on state transitions,
  persisted to the event log, and deliverable by [webhooks](configuration.md) — subscribe an
  endpoint to those event types and a premise death is POSTed to it (signed) the moment the
  sweep detects it. Restoration (`broken → holds`) emits `premise.restored`.
- **Answers:** the monitor watches *stored* claims; the answer-time claim verifier
  (`RAG_CLAIM_VERIFICATION`) independently checks any path an *answer* cites against the
  same listings. An answer that cites a dead path carries its own correction note, whether
  or not the underlying fragment's premise has been swept yet. An answer's path, never seen
  to exist, is reported only when a connected repository holds its first two folders (three
  when the top folder's name starts with `.`, such as `.github/`), so a one-folder path such
  as `scripts/x.sh` is not flagged in answers; a premise that has held
  keeps the first-segment rule below, so it is still caught here.

## The pull-request check

`docbrain check-claims --diff <file|->` asks which live claims a change would falsify, for
the **repository the change is in**. A repository is named by its forge:
`github:owner/repo` or `gitlab:group/project`, GitLab nested groups kept whole
(`gitlab:group/sub/project`), lower-case, host dropped. The CLI makes the name on the
runner and sends only the name — never a URL, which in CI can carry a job token:

1. `--repo <name or clone URL>`;
2. GitHub Actions: `GITHUB_REPOSITORY` (not on Gitea or Forgejo runners, which set
   `GITEA_ACTIONS` / `FORGEJO_ACTIONS`);
3. GitLab CI: `CI_MERGE_REQUEST_PROJECT_PATH` (the merge request's target, also in a fork's
   pipeline), else `CI_PROJECT_PATH`;
4. the `origin` remote (`git remote get-url origin`); an ssh alias is resolved once with
   `ssh -G`;
5. none of these: exit 3, nothing sent.

A web URL is not a name: give the clone URL or the name. A local checkout is tied to a
repository by its own `origin` remote, read at ingest (on github.com, gitlab.com, or the
GitHub or GitLab host the deployment is configured for).

For each live claim on a removed path the server decides from recorded facts only: which
repositories' listings hold the path, and the claiming document's own repository. A claim
about the named repository is a **finding**; a claim about another repository is counted;
anything that cannot be told is listed with its reason and one action — the page's own
repository does not have the path, the page lives in a checkout DocBrain cannot name, a
capture carries no repository while more than one repository holds the path, or DocBrain's
listing does not include the path. A key restricted to spaces sees only those spaces, and
the answer says so.

| Exit | Meaning |
|---|---|
| `0` | Nothing at risk in that repository, or the change removes and renames nothing. |
| `2` | Findings: the change falsifies live claims about this repository. `--warn-only` reports them and exits 0 — unless something could not be told. |
| `3` | Could not run, or could not tell: no repository name; an empty diff on stdin; and, when the change removes or renames a path, a repository DocBrain does not read, claims it cannot tie to a repository, a key that may see no spaces, or a server too old to scope the check. Never 0, `--warn-only` included. |

The diff must be complete, or the check is answering a question nobody asked. A shallow
checkout has no base branch to diff against, `git diff` then fails, and in a pipe without
`pipefail` the check would read nothing. So fetch the history, write the diff to a file
under a shell that stops on error, and pass the file. (`--diff -` with an empty stdin is
refused with exit 3 for the same reason; an empty file means "nothing removed".)

```yaml
# GitHub Actions
- uses: actions/checkout@v4
  with:
    fetch-depth: 0            # the base branch must exist locally
- shell: bash                 # bash -eo pipefail: a failed git diff fails the step
  run: |
    git diff --find-renames --name-status "origin/${{ github.base_ref }}..." > change.diff
    docbrain check-claims --diff change.diff
```

```yaml
# GitLab CI
check-claims:
  rules:
    - if: $CI_PIPELINE_SOURCE == "merge_request_event"
  variables:
    GIT_DEPTH: 0              # the default shallow clone may not hold the target branch
  script:
    - git fetch origin "$CI_MERGE_REQUEST_TARGET_BRANCH_NAME"
    - git diff --find-renames --name-status "origin/$CI_MERGE_REQUEST_TARGET_BRANCH_NAME..." > change.diff
    - docbrain check-claims --diff change.diff
```

Limits, stated:

- Ingest reads a checkout's remote with `git -c safe.directory=<dir>`, because in a
  container the mounted checkout belongs to another user. Git reads that setting from the
  command line from version 2.38; the DocBrain image ships a newer git. On an older git
  outside the image, each such checkout is logged once as "dubious ownership" and stays
  unnamed (its claims are "could not tell", never findings).
- A key restricted to spaces sees only its spaces' claims, but the reasons can still say
  that another repository holds a path (listings carry no space); no claim, count or name
  from another space is shown. Whether a repository name is one DocBrain reads is likewise
  answered for any name the caller supplies.
- The premises page's sweep figures (checked, unsettleable, and how many of the checked
  rows belong to deleted documents) describe the whole corpus, not only the spaces a
  restricted key may see.

## Behaviors worth knowing

These follow from the attribution rules — a premise belongs to sources whose listing contains
its first path segment; a path in an answer, or one a capture declares when it is written, belongs
to sources that hold its first two folders, three under a `.`-led top folder — and are deliberate
trade-offs in favor of precision:

- **Deleting a directory's only file reads as `uncheckable`, not `broken`.** If
  `proof/notes.txt` was the only file under `proof/`, removing it removes the whole
  top-level segment from the listing, so no source owns the claim any more. DocBrain says
  "nothing can speak to this" rather than guessing that a wider deletion was a targeted one.
- **A premise can re-attribute across sources.** If two connected repos both have a `docs/`
  tree, a `docs/...` premise is checked against both, and is `broken` only when an owning
  source decisively lacks it. The answer-time verifier also checks against every owning
  source, but counts as owners only the sources that hold the path's first two folders.
- **Answers and new captures need two folders, premises need one.** A path nobody has seen
  exist (one an answer cites, or one a capture declares when it is written) is reported missing
  only when a connected repository holds its first two folders, because top-level names like
  `docs/` and `scripts/` are shared by most repositories and would otherwise flag files of
  codebases you never connected. A premise that has held was seen to exist, which is better
  evidence of what it is about, so it keeps the first-segment rule and still breaks when its
  one-folder file is deleted.
- **A tool's folder needs a third folder.** A top folder whose name starts with `.` (`.github/`,
  an editor's or an agent's) holds a layout the tool decides, the same in every repository that
  uses it, so for answers and new captures a connected repository must hold the path's first
  three folders. The accepted loss: a file directly under such a two-level folder
  (`.github/workflows/deploy.yml`) that is deleted from a connected repository is not flagged in
  answers. A premise on it that has held still breaks.
- **Precision over coverage, everywhere.** One false "your runbook is wrong" costs more
  than ten missed ones. That asymmetry drives every rule above: deterministic oracles only,
  dated absences, no guessing between candidates, and silence whenever the evidence is
  incomplete.

## What the monitor costs to run

Almost nothing, and document count is the wrong variable to worry about:

- **Documents are not monitored — captured claims are.** A thousand ingested documents
  create zero premises; only knowledge fragments carry them, and only their path-shaped
  claims. Large corpora typically produce hundreds of premises, not thousands.
- **Each check is an in-memory set lookup** — microseconds. A sweep is one small database
  read, one lookup per premise, and writes only for rows whose state or verdict actually
  changed. A sweep over an unchanged world writes nothing and emits nothing. A thousand
  premises is roughly a millisecond of comparison every five minutes; even a hundred
  thousand stays well under a second.
- **The file inventory lives in memory** and refreshes only when your repositories sync.
  The Kubernetes repository's ~31,000 paths measure a few megabytes. The monitor never
  clones anything, never reads document bodies, and never calls a model.

The heavy work — cloning repositories, embedding documents — happens at ingest time, where
it always did. The monitor adds a five-minute heartbeat of list-against-list comparison.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `PREMISE_MONITOR_ENABLED` | `true` | The standing monitor. Turning it off stops sweeps and events; premise rows and the answer-time verifier are unaffected. Helm: `premises.monitorEnabled`. |
| `PREMISE_SWEEP_INTERVAL_SECS` | `300` | Seconds between sweeps. The only dial on sweep cost — each sweep reads and settles every premise row. Values below `5` are raised to `5`. Helm: `premises.sweepIntervalSeconds`. |

Premises need a git source with a recorded file listing — see the GitHub source in
[Configuration](configuration.md). A deployment with no listings runs the monitor inertly:
every premise is `uncheckable` or `dormant`, and nothing alerts.
