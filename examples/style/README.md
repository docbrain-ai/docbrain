# `.docbrain/style.md` example

A runnable example of DocBrain's file-based style policy. Copy the
`.docbrain/` directory into the repository that owns your team's
style policy, then register that repository in DocBrain via
`/admin/policy-file-sources`.

The file under `.docbrain/style.md` is the entire policy — DocBrain's
puller reads it as-is. Do not add commentary or surrounding text to
the policy file itself; the parser expects directives in the
recognized sections (frontmatter, `## Style Overrides`).

For the full guide — layered model, mandatory vs. overridable rules,
branch policy, scheduled pulls, failure modes — see
[`docs/style-policy.md`](../../docs/style-policy.md).
