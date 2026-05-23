---
docbrain_scope: global
---

# DocBrain Public — Style Policy

DocBrain uses this file to dogfood its own file-based puller against
its own documentation repo. A live DocBrain deployment registers
this repository as a policy source for the `global` scope; the puller
fetches this file on a schedule and applies the rules below.

For a user-facing template you can copy into your team's repo, see
[`examples/style/.docbrain/style.md`](../examples/style/.docbrain/style.md)
and the [Style Policy guide](../docs/style-policy.md).

## Style Overrides

- max-heading-depth: 4
- max-sentence-length: 30
- avoid-word: just
- avoid-word: simply
- avoid-word: obviously
