# DocBrain Template Files

**A template is just a markdown file your team already has.** Point `--template`
at an existing runbook, a doc skeleton, or any document whose layout you want new
docs to match. **There is no special format to learn** — `docbrain generate`
reads the file's `## Section` headings (in order) plus an optional tone, and
writes a new doc with **that structure and voice**.

The files in this folder are ready-to-copy starting points, but **any ordinary
`.md` works** — see [`runbook.md`](runbook.md) for one with the optional
`doc_type:`/`tone:` lines, or just hand `generate` a doc you already maintain.

```bash
# Use a doc the team already wrote as the shape for a new one — no edits needed
docbrain generate "runbook for cert rotation" \
  --template docs/runbooks/EXISTING-runbook.md \
  --source notes.md > out.md
```

## What a template can and cannot do

A template controls **structure and tone only**.

- It **can** add, remove, reorder, and rename the required sections for a run.
- It **can** set the writing tone (`concise and operational`, `welcoming and
  step-by-step`, …).
- It does **not** copy your file's prose into the output. `generate` takes the
  *skeleton + tone* and writes fresh, grounded content under each heading. (Want
  the output to reuse a document's actual text? Pass it as a `--source` seed, not
  a `--template`.)
- It **cannot** carry, weaken, or disable any quality or safety behaviour. The
  format simply has no field for one. Generated docs always pass the same
  redaction, scrubbing, and scoring gates regardless of the template used.

A template is structure-and-tone guidance, not policy. Don't try to put rules in
it (see "Format rules" below) — rule-like lines are rejected.

## Format rules

| Rule | Detail |
|------|--------|
| `doc_type:` line | Optional. Leading metadata line. One of `runbook`, `guide`, `troubleshooting`, `faq`, `reference`. |
| `tone:` line | Optional. Free text describing the desired voice. |
| `## Section` headings | Each level-2-or-deeper heading becomes a **required** section. Order is preserved; duplicates are de-duped. |
| Prose / bullets / blank lines | **Ignored** by the engine. They're notes for whoever edits the template — use them freely to explain what each section is for. |
| Unknown metadata lines | **Ignored**, not an error. A line like `audience: SREs` is harmless guidance, not a directive. |
| Empty / heading-less file | Valid. Means "no structure constraint," not an error. |
| Rule-like `key: value` lines | **Rejected** (HTTP 400). A line whose key is one of `rules`, `rule`, `safety`, `secret(s)`, `redaction`/`redact`, `quality_rule(s)`, `policy`/`policies`, `allow`, `deny`, `disable`, `override(s)` (e.g. `safety: off`, `disable: redaction`) is refused — you cannot smuggle config through a structure-only file. Everything outside that small denylist is ignored. |

Because prose under a heading is ignored, every template in this folder puts a
short one-line note under most sections explaining what belongs there. That text
never reaches the generated document — it only helps the next person who edits
the template.

## How to use one

Point `--template` at the file:

```bash
docbrain generate "runbook for cert rotation" \
  --template examples/templates/runbook.md \
  --source notes.md > out.md
```

The template's sections and tone shape the run; your sources and your org's
knowledge fill them in. `stdout` is pipe-clean markdown — all diagnostics go to
`stderr`, so the redirect above is safe.

Over the API, pass the **file's contents** (not its path) in the `template`
field of the `POST /api/v1/generate` request body.

## Seeded doc-type section sets

If you don't supply a template, `docbrain generate` uses these built-in section
sets for the resolved doc type. A `--template` file overrides or extends them
for that run.

| Doc type | Required sections | Optional sections |
|----------|-------------------|-------------------|
| `runbook` | Overview, Prerequisites, Steps, Verification, Rollback | — |
| `guide` | Overview, Prerequisites, Steps | FAQ, Related Guides, Next Steps |
| `troubleshooting` | Symptoms, Diagnosis, Resolution | — |
| `faq` | free-form (no required sections) | — |
| `reference` | Overview, Parameters, Examples | — |

The files in this folder are real, copy-pasteable starting points built on top
of these defaults:

| File | Doc type | Shape |
|------|----------|-------|
| `runbook.md` | `runbook` | Operational runbook skeleton |
| `troubleshooting.md` | `troubleshooting` | Symptom → diagnosis → fix |
| `api-reference.md` | `reference` | Endpoint/API reference with auth, errors, changelog |
| `onboarding-guide.md` | `guide` | Friendly step-by-step onboarding |

Copy one, adjust the headings and tone to fit your team, and pass it with
`--template`.
