# Style Policy

DocBrain's style policy is the set of rules that govern how documents
look — what words to avoid, what sections every runbook must have, how
deep heading nesting can go. The policy is enforced at three points:

1. **Draft generation** — the LLM is prompted with the effective ruleset
   for the target space, so generated drafts are born compliant.
2. **Post-generation lint** — every draft is linted against the same
   ruleset before it lands in review. Mandatory-severity violations
   block publish unless an admin explicitly overrides.
3. **Existing-content lint** — `/api/v1/quality/lint` and the Quality
   Score page surface rule violations in the corpus, so operators can
   prioritise cleanup.

This page covers how the policy is structured, where it comes from, and
what an operator needs to know about how changes propagate. For the
API surface (CRUD endpoints, request/response shapes), see the
[API Reference](api-reference.md#style-rules-engine).

## Layered model

DocBrain rules sit at two scopes:

| Scope            | Applies to                                            | Set by                       |
| ---------------- | ----------------------------------------------------- | ---------------------------- |
| `global`         | Every draft and document in the corpus                | Admin (UI / API / YAML import) |
| `space_override` | Every draft and document in one specific space        | Admin (UI / API / file-based puller) |

When a draft is generated for a space, DocBrain computes the
**effective ruleset** for that space:

```
effective_rules(space) = globals ∪ (space_overrides − keys_already_mandatory)
```

A space override may shadow an overridable global by the same
`(rule_type, name)` identity. It may **not** shadow a mandatory global —
the resolver drops the override and logs the attempt. This is how
admins encode "every team can tune the heading depth, but the
brand-mandated terminology rule is non-negotiable."

## Mandatory vs. overridable

Every rule has an `enforcement_level`:

- **`overridable`** (default) — a space can override this rule.
  Teams have flexibility; the global is a baseline.
- **`mandatory`** — a space cannot override this rule. The puller
  rejects any file-based attempt to weaken it; the resolver drops any
  in-memory override attempt; the DB has a trigger that raises an
  exception on a direct `DELETE` of a mandatory row. This is reserved
  for org-wide policies that must hold everywhere (e.g. PII-language
  rules, compliance terminology).

Mandatory rules can only be created or modified through the admin API
by an admin role. The file-based puller is intentionally limited to
overridable rules.

## Where rules come from

There are three sources of style rules. They all write to the same
underlying table; the difference is who controls them.

### 1. Admin UI (`/admin/style-rules`)

Hand-crafted rules created by an administrator. Useful for one-off
rules, mandatory rules, and rules with `severity=error` that block
publish.

### 2. YAML import (`POST /api/v1/style-rules/import`)

Bulk-import a ruleset from a YAML file. Useful for seeding a new
deployment from a curated template, or for keeping a version-controlled
copy of the ruleset checked into a config repo.

### 3. File-based puller (`.docbrain/style.md`)

The puller monitors a registered source repository and syncs the rules
declared in its `.docbrain/style.md` file on a schedule. This is the
"GitOps for style policy" path — the policy file lives in the repo
that owns the team's standards, change review goes through normal
PR review, and DocBrain pulls the latest version.

See the [working example](https://github.com/docbrain-ai/docbrain-public/tree/main/examples/style)
for a complete `.docbrain/style.md` you can copy.

#### Registering a source

Admin → **Style Policy** in the sidebar (or `/admin/policy-file-sources`).
Click "Register a source" and provide:

- **Repository URL** — the HTTPS URL of the repo containing
  `.docbrain/style.md` at its root.
- **Declared scope** — the space this repo is authorized to publish
  policy for. Use a space name (e.g. `ENG`) for space-scoped rules, or
  `global` for org-wide overrides.

The puller verifies that the file's frontmatter `docbrain_scope` matches
the registered scope before applying any rules — a repo registered for
`ENG` cannot smuggle rules into `HR`.

#### Branch policy (v1)

The puller fetches `.docbrain/style.md` from the **repository's default
branch only**. This is intentional: the trust model assumes the
customer has branch protection on the default branch, so policy
changes go through review. Feature branches typically don't have the
same protection, so accepting non-default refs would let any team
member with push access deploy policy bypassing review.

Non-default branch support is on the roadmap with additional
audit-trail safeguards.

#### Scheduled pulls

The scheduler runs every `POLICY_FILE_SYNC_INTERVAL_SECS` seconds
(default 900s = 15 minutes). Each tick sweeps every `enabled=true`
source, fetches its `.docbrain/style.md`, parses, and upserts the
declared rules. Per-source failures are isolated — a 404 or a transient
network error on one repo doesn't abort the sweep, and the failure
surfaces on that source's row as `last_error`.

You can disable the scheduler by setting `POLICY_FILE_SYNC_INTERVAL_SECS=0`.
Manual pulls via the admin "Sync now" button still work when the
scheduler is disabled.

## How rule changes propagate

This is the part operators most often ask about.

**Single-replica deployments.** When an admin creates, updates, or
deletes a rule via the API or UI, the server's in-memory rule cache
reloads immediately after the database commit. The next draft request
sees the new rule. There is no cache TTL.

**Multi-replica deployments.** Each replica has its own in-memory rule
cache. When an admin's PATCH lands on Replica A, **only Replica A
reloads** — Replica B's cache stays stale until one of:

- Replica B handles its own write (the next admin action that lands on it)
- Replica B's scheduled-pull tick processes a source and triggers a reload
- Replica B restarts

There is currently no cross-replica invalidation broadcast. This is a
known limitation. Operators running multi-replica deployments who need
guaranteed cross-pod consistency should restart the deployment after
any policy change until cross-replica invalidation ships.

## What's recorded for every draft

When a draft is generated, DocBrain freezes a snapshot of the rules
that were effective at that moment into `autopilot_drafts.style_snapshot`.
This means:

- An audit of any past draft can show exactly which rules applied.
- A rule change in the future doesn't retroactively re-judge an old
  draft.
- The `severity` and `enforcement_level` of each rule at the time are
  preserved.

## Failure modes

The puller has explicit semantics for every failure class. None of these
silently corrupt your ruleset.

| Failure                              | Effect                                                                     |
| ------------------------------------ | -------------------------------------------------------------------------- |
| Network / 5xx / timeout              | Existing rules left in place. `last_error` set.                            |
| 401 / 403 (auth)                     | Existing rules left in place. `last_error` set.                            |
| 404 (file deleted upstream)          | Rules previously pulled from this source are deleted. (Team retracted policy.) |
| Oversized file                       | Existing rules left in place. `last_error` set.                            |
| Scope mismatch (file says HR, registered as ENG) | Existing rules left in place. **Logged as security event.** `last_error` set. |
| Tries to define a mandatory rule     | Existing rules left in place. **Logged as security event.** `last_error` set. |

The two security-event rows (scope mismatch and mandatory smuggling)
are the load-bearing trust boundaries of the file-based puller. They
ensure a compromised or hostile policy file cannot escalate a rule's
authority or cross scope boundaries.

## Configuration reference

| Setting / env var                                         | Default       | Effect                                                                  |
| --------------------------------------------------------- | ------------- | ----------------------------------------------------------------------- |
| Setting `style.policy_file_max_bytes`                     | `262144` (256 KiB) | Maximum file size the puller will accept. Larger files are rejected. |
| Env `POLICY_FILE_SYNC_INTERVAL_SECS`                      | `900` (15 min) | Scheduled-pull interval in seconds. `0` disables the scheduler.        |

For the full set of style-rule-related API endpoints, see the
[API Reference](api-reference.md#style-rules-engine).
