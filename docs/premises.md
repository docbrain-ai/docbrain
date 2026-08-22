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

## Where premises come from

1. **Automatic extraction.** When a fragment is indexed, path-shaped tokens in inline code
   spans (a filename with an extension, e.g. `` `scripts/rotate.sh` ``) are extracted as
   premises. Prose mentions, globs, URLs, flags, and placeholders are ignored — the same
   grammar the [answer-time claim verifier](configuration.md) uses.
2. **Explicit declaration.** Agents capturing over MCP can pass a `premises` array to
   `docbrain_annotate` — see [Agent Capture](agents.md). v1 checks `premise_type: "path"`;
   other types are recorded but never checked (and never alert). Malformed entries are
   skipped item-by-item — a bad premise never fails the capture.
3. **Backfill.** `POST /api/v1/premises/backfill` (admin) extracts premises for every
   already-indexed fragment that has none. Safe to re-run; it never duplicates.

## The four states

| State | Meaning | Alerts? |
|---|---|---|
| `holds` | Verified true against a connected source's listing | — |
| `broken` | Was verified true, now decisively absent | **yes** — `premise.broken` |
| `uncheckable` | No connected source can currently speak to it | never |
| `dormant` | Never verified true | never |

Two rules keep this honest:

- **Only a claim proven true can die.** A premise that fails verification at capture is
  born `dormant` and stays silent forever — so a placeholder path in an example can never
  page anyone. If a later check finds a dormant premise true, it is promoted to `holds`
  (silently) and monitored from then on.
- **"I can't check" is not "it's gone."** If a premise's source is disconnected or its
  listing pruned, the premise becomes `uncheckable` — an infrastructure fact, not an alarm.
  It keeps its last verdict and basis, so you can see what was known and when. When the
  source returns, the next sweep re-verdicts it; only a genuine death alerts.

## The monitor

With `PREMISE_MONITOR_ENABLED=true` (the default), the server runs a standing monitor:

- **Every 300 seconds** it sweeps all premises against the current file listings.
- **On fragment-index events** it extracts and initially verifies a new fragment's premises
  immediately, rather than waiting for the sweep.
- **At startup, and after any missed events,** it backfills premises for indexed fragments
  that have none, so nothing is silently unmonitored.

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
  `GET /api/v1/premises/health` (analyst+), `POST /api/v1/premises/backfill` (admin).
- **Events:** `premise.broken` and `premise.restored` are emitted on state transitions,
  persisted to the event log, and deliverable by [webhooks](configuration.md) — subscribe an
  endpoint to those event types and a premise death is POSTed to it (signed) the moment the
  sweep detects it. Restoration (`broken → holds`) emits `premise.restored`.
- **Answers:** the monitor watches *stored* claims; the answer-time claim verifier
  (`RAG_CLAIM_VERIFICATION`) independently checks any path an *answer* cites against the
  same listings. An answer that cites a dead path carries its own correction note, whether
  or not the underlying fragment's premise has been swept yet.

## Behaviors worth knowing

These follow from the attribution rule — a premise belongs to sources whose listing contains
its first path segment — and are deliberate trade-offs in favor of precision:

- **Deleting a directory's only file reads as `uncheckable`, not `broken`.** If
  `proof/notes.txt` was the only file under `proof/`, removing it removes the whole
  top-level segment from the listing, so no source owns the claim any more. DocBrain says
  "nothing can speak to this" rather than guessing that a wider deletion was a targeted one.
- **A premise can re-attribute across sources.** If two connected repos both have a `docs/`
  tree, a `docs/...` premise is checked against both, and is `broken` only when an owning
  source decisively lacks it. This is the same any-owning-source semantics the answer-time
  verifier uses.
- **Precision over coverage, everywhere.** One false "your runbook is wrong" costs more
  than ten missed ones. That asymmetry drives every rule above: deterministic oracles only,
  dated absences, no guessing between candidates, and silence whenever the evidence is
  incomplete.

## Configuration

| Variable | Default | Description |
|---|---|---|
| `PREMISE_MONITOR_ENABLED` | `true` | The standing monitor. Turning it off stops sweeps and events; premise rows and the answer-time verifier are unaffected. Helm: `premises.monitorEnabled`. |

Premises need a git source with a recorded file listing — see the GitHub source in
[Configuration](configuration.md). A deployment with no listings runs the monitor inertly:
every premise is `uncheckable` or `dormant`, and nothing alerts.
