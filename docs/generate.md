# Generate — Grounded Document Generation

`docbrain generate` produces a documentation draft grounded in your org's own
knowledge — your corpus, your past episodes, and your live MCP connectors —
and returns the markdown to you. It does **not** publish anything; it is
stateless. The moat is grounding: where a frontier model writes fluent generic
prose, `generate` writes what is actually true for *your* org (with per-claim
provenance), passes every DocBrain quality and safety gate, and is honest when
it doesn't know rather than fabricating.

## When to use it

`generate` is the **on-demand, you-drive-it** counterpart to autopilot.

- **Autopilot** is automatic and gap-driven: DocBrain detects a documentation
  gap and proposes a draft on its own schedule. You don't ask for a specific
  document — the system decides what's missing.
- **Generate** is on-demand: *you* name the document you want, *you* hand it the
  primary source material, and *you* get the markdown back to do with as you
  please. It runs exactly when you invoke it and produces exactly what you asked
  for.

And versus a plain LLM:

| | Plain frontier model | `docbrain generate` |
|-|-|-|
| Knows your runbooks, incidents, tickets, PRs, Slack threads | No | Yes — grounded in your corpus + episodes + live connectors |
| Per-claim provenance | No | Yes — every claim attributed to a source |
| Passes redaction / PII / hostname / injection gates | No | Yes — same gates as every DocBrain doc |
| Behaviour when it doesn't know | Fabricates fluent prose | Emits `needs_input` and says it can't |

!!! note
    A frontier model with no access to your systems writes something that
    *reads* right. `generate` writes something that *is* right for you — or
    tells you which questions it can't answer.

## CLI

```
docbrain generate <ASK> [FLAGS]
```

`<ASK>` is the single required positional argument: a plain-language description
of the document you want written.

### Flags

| Flag | Value | Meaning |
|-|-|-|
| `<ASK>` | string (positional) | **Required.** What to write. |
| `--source` | `<FILE>` | Repeatable. A local file read verbatim as **primary** material. |
| `--source-url` | `<URL>` | Repeatable. A **link** as primary material; DocBrain **fetches** it via the connected MCP connector. See [Source material](#source-material). |
| `--stdin` | — | Read primary material from stdin. |
| `--target` | `<REF\|URL>` | Existing doc to **augment** (not replace). See [Augmenting an existing doc](#augmenting-an-existing-doc). |
| `--template` | `<FILE>` | Team template file that shapes sections and tone. **Cannot** affect safety. See [Templates](#templates). |
| `--type` | `<TYPE>` | Doc-type hint: `runbook` \| `guide` \| `troubleshooting` \| `faq` \| `reference`. Inferred if omitted. |
| `--space` | `<SPACE>` | Confluence space whose quality rules apply. Falls back to the global floor if omitted. |
| `--out` | `<FILE>` | Write markdown to a file instead of stdout. |
| `--no-enrich` | — | Disable live-MCP tool enrichment (corpus/seed only). |
| `--allow-violations` | — | Exit `0` even on error-severity quality violations (CI override). |

### I/O contract

- **stdout carries only the markdown.** It is pipe-clean and redirect-safe — you
  can `> out.md` or pipe into another tool without stripping noise.
- **All diagnostics go to stderr**: resolved doc type, quality score,
  needs-input questions, skipped sources, and quality violations.
- **Exit code is non-zero on error-severity quality violations** — unless you
  pass `--allow-violations`. This is the CI-native behaviour: a bad draft fails
  the build by default.

```bash
# Runbook from local notes, redirected to a file (pipe-clean stdout)
docbrain generate "runbook for cert rotation" --source notes.md > out.md
```

## Source material

You provide **primary** material in one of three ways. All three are repeatable
and can be combined.

| Kind | Flag | What it is |
|-|-|-|
| File | `--source <FILE>` | A local file, read verbatim. |
| Stdin | `--stdin` | Material piped in on standard input. |
| URL | `--source-url <URL>` | A link DocBrain fetches for you via the connected MCP connector. |

### `--source-url`: supported links

A `--source-url` is an explicitly **named** primary source. DocBrain resolves it
through the connected MCP connector and reads the content for you:

- **Confluence page** — `…/wiki/…/pages/<id>` or `…/wiki/spaces/…`
- **Jira issue** — `…/browse/KEY-123`
- **Slack thread** — `…/archives/C…/p…`
- **GitHub PR** — `…/pull/<n>`
- **GitHub file** — `…/blob/<ref>/<path>`

```bash
# Postmortem grounded in a real Slack incident thread
docbrain generate "postmortem from this incident" \
  --source-url https://acme.slack.com/archives/C123/p1700000000123 > postmortem.md

# API reference grounded in a GitHub PR's changes
docbrain generate "API reference for the changed endpoints" \
  --source-url https://github.com/acme/repo/pull/42 --type reference
```

### All-or-nothing hard-fail contract

Because a `--source-url` is a source you explicitly named, it is treated as
load-bearing. **If any one of them can't be fetched, the whole generation
aborts** — naming the failed source. It will never silently produce a document
from a subset of the sources you requested.

| Failure | Code |
|-|-|
| Connector not connected / not configured | `502` |
| Fetch error | `502` |
| Unsupported or unrecognized link | `400` |

Multiple `--source-url` flags are allowed, and **all must succeed**.

!!! warning "Named sources vs. auto-enrichment"
    The hard-fail contract applies **only** to sources you named with
    `--source-url`. Auto-enrichment — the live-MCP context DocBrain pulls on its
    own — is **best-effort**: a connector being down just means *less context*,
    not a failed run. Named sources fail loud; enrichment degrades quietly.

URL-fetched content is size-bounded exactly like inline sources (per-source
**and** aggregate byte caps), so a huge linked file or thread can't overflow the
prompt. Over-budget material returns `413`.

## Templates

A template is a plain markdown/text file that shapes a document's **structure**
and **tone** — nothing else. It can **never** carry or disable a safety or
quality rule: the parser has literally no field for one.

### Format

- An optional leading `doc_type: <type>` metadata line.
- An optional leading `tone: <free text>` metadata line.
- Every level-2-or-deeper markdown heading (`## Section`) becomes a **required
  section**, in document order. Duplicates are de-duped; order is preserved.
- **All other text is ignored** by the engine — prose, bullets, and blank lines
  are guidance for the human writing the template, not consumed by generation.
  An unknown metadata line such as `audience: SREs` is simply ignored, not an
  error.
- An empty or section-less file is **fine** (no structure constraint) — not an
  error.
- The **only** error case: a line whose key is a *rule-like* directive — one of
  `rules`, `rule`, `safety`, `secret(s)`, `redaction`/`redact`,
  `quality_rule(s)`, `policy`/`policies`, `allow`, `deny`, `disable`,
  `override(s)` (e.g. `safety: off`, `disable: redaction`). The engine
  **rejects it with `400`**. You cannot smuggle safety config through a
  structure-only template; everything outside that small denylist is ignored.

### Example template file

```text
doc_type: runbook
tone: concise and operational

## Overview
## Prerequisites
## Steps
## Rollback
```

### Seeded doc-type section sets

Each doc type ships with a built-in section set. A `--template` file
overrides/extends these for the run.

| Doc type | Required sections |
|-|-|
| `runbook` | Overview, Prerequisites, Steps, Verification, Rollback |
| `guide` | Overview, Prerequisites, Steps *(optional: FAQ, Related Guides, Next Steps)* |
| `troubleshooting` | Symptoms, Diagnosis, Resolution |
| `faq` | Free-form (no required sections) |
| `reference` | Overview, Parameters, Examples |

### Usage

```bash
docbrain generate "runbook for cert rotation" \
  --source notes.md \
  --template templates/acme-runbook.md > out.md
```

## Augmenting an existing doc

`--target <REF|URL>` points at an existing document and tells `generate` to
**augment** it — fill gaps, add sections, fold in new source material — rather
than rewrite it from scratch. The existing content is the baseline; the run adds
to it.

```bash
# Extend an existing runbook with what a new PR changed
docbrain generate "add the new rollback path" \
  --target https://acme.atlassian.net/wiki/spaces/OPS/pages/12345 \
  --source-url https://github.com/acme/repo/pull/42
```

## API

```
POST /api/v1/generate
```

Editor role; same auth as `/ask`. Returns `200` with a `GeneratedArtifact`.

### Request body — `GenerateRequest`

```json
{
  "ask": "string (required)",
  "sources": [
    { "kind": "file", "label": "notes.md", "raw": "<file bytes>" },
    { "kind": "url",  "label": "https://github.com/acme/repo/pull/42", "raw": "" }
  ],
  "target": "optional existing-doc reference to augment",
  "template": "optional RAW template file CONTENT (the bytes, not a path)",
  "doc_type": "optional hint",
  "space": "optional confluence space",
  "no_enrich": false
}
```

- For `kind: "url"`, the URL goes in `label` and `raw` is `""` — **the server
  fetches it**.
- `template` is the raw template **file content** (the bytes), not a path.

### Response — `GeneratedArtifact`

```json
{
  "markdown": "the generated document",
  "doc_type": "resolved type",
  "provenance": [
    { "section": "Overview", "source_ids": ["chunk-1", "chunk-2"] }
  ],
  "needs_input": [ "questions the doc can't answer from available knowledge" ],
  "skipped_sources": [
    { "label": "jira", "reason": "connector not connected" }
  ],
  "quality": {
    "score": 87.0,
    "violations": [
      { "rule_name": "...", "severity": "warning", "message": "..." }
    ]
  }
}
```

| Field | Meaning |
|-|-|
| `markdown` | The generated document. |
| `doc_type` | The resolved doc type (from `--type`/`doc_type` or inferred). |
| `provenance` | Per-section source attribution. Each entry is `{ section, source_ids }` — `section` is the heading (`null` = whole-doc), `source_ids` the evidence chunks it was built from. |
| `needs_input` | Questions the doc **can't** answer from available knowledge — the honesty signal. |
| `skipped_sources` | Sources that were unavailable, each as `{ label, reason }` (e.g. `{ "label": "jira", "reason": "tool not connected" }`). |
| `quality` | `{ score, violations }` — `score` is a 0–100 number; each violation is `{ rule_name, severity, message }`. |

### Error codes

| Code | Meaning |
|-|-|
| `400` | Bad request / unknown kind / unsupported URL (incl. a smuggled rule directive in a template). |
| `403` | Caller is not an editor. |
| `413` | Source material is over the size budget. |
| `502` | A named URL source could not be fetched (connector not connected, not configured, or fetch error). |
| `503` | Generation is not configured. |

## Using `generate` in CI

`docbrain generate` is the one doc command worth putting in a pipeline, because
it is the only one that writes documentation **grounded in your org's real
knowledge** — your runbooks, past incidents, tickets, PRs, and Slack threads —
with per-claim provenance, and that is **honest** about what it doesn't know (it
emits `needs_input` instead of fabricating). In CI that turns into a concrete
property: documentation updates ship *as a side effect of merging code*,
grounded and quality-gated, with zero extra human work.

The three playbooks below cover the common shapes. They use `acme` as a
placeholder org — swap in your own connector URLs and spaces.

### Playbook 1 — Doc-on-merge: update the runbook from the PR

The headline case. When a PR lands, regenerate the affected runbook from the PR
itself plus the existing doc, and hand an engineer a review-ready draft.

```yaml
# .github/workflows/docs-on-merge.yml
name: Update runbook on merge
on:
  pull_request:
    types: [closed]
    paths: ['services/payments/**']

jobs:
  update-runbook:
    if: github.event.pull_request.merged == true
    runs-on: ubuntu-latest
    env:
      PR_NUMBER: ${{ github.event.pull_request.number }}
      DOCBRAIN_TOKEN: ${{ secrets.DOCBRAIN_TOKEN }}
    steps:
      - name: Generate runbook draft grounded in the PR
        run: |
          docbrain generate "update the deploy runbook for the changes in this PR" \
            --source-url https://github.com/acme/payments/pull/${PR_NUMBER} \
            --target https://acme.atlassian.net/wiki/spaces/SRE/pages/12345/Deploy+Runbook \
            --type runbook --allow-violations --out runbook-draft.md
```

What this actually does:

- **Fetches the PR through the connected GitHub connector.** The `--source-url`
  link is resolved server-side via the MCP connector you already configured —
  the runner needs no GitHub credentials of its own for the fetch, just a
  DocBrain token.
- **Grounds the draft in *both* the PR and your org corpus.** The model sees the
  PR diff *and* the existing runbook (via `--target`) *and* whatever
  auto-enrichment pulls from related incidents, tickets, and threads. The output
  is what is true for *acme's* deploy, not generic prose.
- **Augments — it does not overwrite.** `--target` treats the existing page as
  the doc being revised, so structure and prior content are preserved and the
  draft is a coherent next version, not a from-scratch rewrite.
- **Every claim carries provenance.** The draft and the API `provenance[]` array
  trace assertions back to the sources they came from, so a reviewer can verify
  rather than trust.

The `runbook-draft.md` artifact is the input to a human review step — for
example, post it as a PR comment or open a docs PR:

```yaml
      - name: Post the draft for review
        run: gh pr comment ${PR_NUMBER} --body-file runbook-draft.md
```

This playbook uses `--allow-violations` deliberately: a merge-time draft is
advisory and a human gates it, so you want the draft even if it trips a quality
rule. Playbook 3 is where you flip that.

### Playbook 2 — Diff-driven: generate from a git diff file

Not every pipeline has a PR URL to pass — pre-merge jobs, GitLab/Jenkins/
Buildkite runs, and local pre-commit checks often only have the working tree. In
that case, hand `generate` the diff on disk with `--source`:

```yaml
      - name: Generate API notes from the diff
        run: |
          git diff origin/main...HEAD > changes.diff
          docbrain generate "document the behavior change in these endpoints" \
            --source changes.diff --type reference --out api-notes.md
```

When to use which form:

| Use `--source-url <PR>` (Playbook 1) | Use `--source <diff>` (Playbook 2) |
|---|---|
| You have a merged/open PR URL | You only have the working tree |
| GitHub, with the connector configured | Any CI system — no connector needed for the diff |
| Post-merge "doc the shipped change" | Pre-merge "doc the proposed change" |
| Connector also pulls PR comments/reviews | Diff is exactly what's on disk, nothing more |

`--source` is a plain local file read by the runner, so it works anywhere `git`
runs and needs no connectivity to your wiki for the diff itself. Auto-enrichment
still reaches into your org corpus for grounding context unless you pass
`--no-enrich`.

### Playbook 3 — Quality gate: fail the build on a bad doc

Drop `--allow-violations` and `generate` becomes a gate. The CLI **exits
non-zero on error-severity quality violations**, so the doc must clear the bar
or the build breaks.

```yaml
      - name: Generate and gate the troubleshooting doc
        run: |
          docbrain generate "document the new retry/backoff behavior" \
            --source changes.diff --type troubleshooting \
            --space SRE --out incident-doc.md
```

If the draft fails a quality rule at error severity, the step's exit code is
non-zero and the job fails — no human in the loop, the pipeline simply won't
pass a sub-bar doc. The stdout/stderr contract is what makes this safe to script:

- **stdout is *only* the markdown.** Pipe-clean and redirect-safe — `>
  incident-doc.md` captures the document and nothing else. No banners, no
  progress logs.
- **stderr carries every diagnostic.** Doc type, quality score, the `needs_input`
  list, `skipped_sources`, and the specific violations all land on stderr where
  your CI logs collect them without polluting the artifact.
- **The exit code is the CI signal.** A zero exit means the doc met the bar,
  non-zero means it didn't — you don't parse logs to decide pass/fail.

`--space <SPACE>` applies that Confluence space's quality rules to the run, so
the doc is held to the *same* standard your space enforces for human-authored
pages — in CI, before anyone sees it.

### Why this beats a generic LLM action

A frontier model dropped into a generic GitHub Action writes *plausible* prose.
`docbrain generate` writes what is *true for your org* — or says it can't:

- **Grounded in org reality, with provenance.** It reads your actual PRs,
  runbooks, incidents, tickets, and threads through your connectors, and every
  claim traces back to a source in `provenance[]`. A generic action has no
  access to any of that and invents the specifics.
- **Honest `needs_input` instead of fabrication.** When the corpus doesn't cover
  something, `generate` flags it in `needs_input[]` rather than confidently
  making it up — you fix gaps on purpose instead of discovering an invented
  procedure during an incident.
- **The same safety gates as every other DocBrain doc, enforced in CI.** Secret
  and PII redaction, hostname scrubbing, and prompt-injection quarantine run on
  the generated draft exactly as for human-authored content — your pipeline
  can't accidentally publish a leaked credential or a poisoned source.
- **Quality-gated by exit code.** Without `--allow-violations`, a sub-bar doc
  fails the build. A generic LLM action always exits zero and ships whatever it
  produced; DocBrain makes "good enough to publish" a machine-checkable
  condition.

## Safety

A generated document passes the **same gates as every other DocBrain doc** —
there is no weaker path through `generate`:

- **Secret / PII redaction** — credentials and personal data are stripped.
- **Hostname scrub** — internal hostnames are removed.
- **Injection quarantine** — prompt-injection content in fetched/linked sources
  is quarantined, not executed.
- **Structural + style scoring** — the `quality.score` and `violations` that
  drive the CI exit code.

**Templates cannot weaken safety.** A template shapes structure and tone only;
the parser has no field for a safety or quality rule, and any rule-like
directive is rejected with `400`. There is no template knob that turns off
redaction, the hostname scrub, or the injection quarantine.

**`needs_input` honesty.** When the document can't answer something from the
available knowledge, `generate` says so — it emits the open questions in
`needs_input` rather than fabricating a confident-sounding answer. Honest gaps
over fluent fiction.
