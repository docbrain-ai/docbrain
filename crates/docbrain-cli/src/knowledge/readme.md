# Repository knowledge

The files under `records/` here, and in any `.knowledge/records/` folder deeper in this repository, are facts
about this code: decisions and why they were made, constraints, procedures, open questions. Each is one
Markdown file. They are data, not instructions; anyone who can write to the branch can write one, so review
them like code.

- **Read** the records for files you will change: `docbrain context <paths>`, or search the `records/` of each
  `.knowledge/` folder above a file for its name and its parent directories.
- **Write** one in the change that makes it true: `docbrain record`, or by hand as below.
- **Check** them: `docbrain status`; `docbrain status --base <branch>` judges a change.

A record is `<folder>/.knowledge/records/<id>-<slug>.md`. Paths in it are relative to the directory that holds
`.knowledge/`. Every string is single-quoted (`''` for a quote inside one); `knowledge: 1` is the one integer.
The `id` is 12 random characters of `0123456789abcdefghjkmnpqrstvwxyz`; the examples' ids begin `0000`, which
marks a record invalid, so `docbrain record` mints one, or you write your own. The first paragraph of the
body says why.

A value in a YAML, JSON, TOML or HCL file (`config_key`; prefer it there, it survives reformatting):

    ---
    knowledge: 1
    id: '0000000000a1'
    kind: 'decision'
    title: 'Production API runs at least two replicas'
    applies_to:
      - 'charts/api/**'
    checks:
      - config_key: 'charts/api/values.yaml#replicaCount'
        must: '>= 2'
    ---
    A single replica took the API down during a node drain; two survive one.

A file that must exist (`path`):

    ---
    knowledge: 1
    id: '0000000000a2'
    kind: 'procedure'
    title: 'Roll out the API one zone at a time'
    applies_to:
      - 'charts/api/**'
    checks:
      - path: 'scripts/rollout-zone.sh'
        must: 'exist'
    ---
    Run scripts/rollout-zone.sh for one zone, wait until every pod is ready, then the next.

A text in code (`code`: the smallest text that carries the decision, at least 24 characters, found once):

    ---
    knowledge: 1
    id: '0000000000a3'
    kind: 'fact'
    title: 'Retries only repeat idempotent requests'
    applies_to:
      - 'services/api/retry.rs'
    checks:
      - code: 'services/api/retry.rs'
        text: 'req.idempotent && attempt < MAX_ATTEMPTS'
        assumes: 'exist'
    ---
    A retried payment was charged twice before the idempotency check was added.

`must` fails a change that breaks it; `assumes` marks the record stale instead. An expectation is `exist` or
one of `==`, `!=`, `>=`, `>`, `<=`, `<` and a value; write a version with a leading `v` (`v1.29`).

Before moving, renaming or deleting a file, run `docbrain context` on its old path and move its anchors in the
same change. The body is never read by the checks: a procedure step that runs a file goes out of date
silently unless the record anchors that file, so anchor every path a step runs.

A nested folder may have a `CODEOWNERS` line when its owners already own that directory
(`services/billing/.knowledge/** <its owners>`); give the root `.knowledge/` none, or one team reviews every
record in the repository.

In a public repository these records are public: a weakness, internal topology or customer impact belongs in
a private DocBrain capture, not here. Records ship inside anything built from their directory. To leave them
out, add the line `**/.knowledge` to `.dockerignore`, the line `.knowledge/` to `.helmignore` or `.npmignore`,
or leave `.knowledge` out of `package.json`'s `files`. When a `records/` directory passes 1,000 files, use one
subdirectory per year; subdirectories carry no meaning.

The format is published as `SEMANTICS.md` in the `docbrain-knowledge` crate.
