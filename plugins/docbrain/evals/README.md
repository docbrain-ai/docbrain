# Evals for the docbrain plugin

Deterministic checks of what the `docbrain-capture` skill makes Claude do, run by `claude plugin eval`.
The DocBrain connector is mocked under `mocks/docbrain/`, so the suite needs no server and no key,
answers the same way every run, and can gate a change in CI. Each case is a realistic conversation;
its graders check which connector tools were called and with what, and what the reply said.

Run from the repository root:

    claude plugin eval plugins/docbrain --scaffold --trust-plugin --ablation none

`--scaffold` lets the cases that need a repository build one first. Add `--json results.json
--threshold 1.0` in CI. The live end-to-end proofs against a real stack are separate:
`scripts/capture-skill-proof.sh` and `scripts/capture-skill-proof-b.mjs`.

`mocks/docbrain/_tools.json` is the real server's `tools/list` answer for the six tools the skill
uses, so the model under test sees their real descriptions and input schemas (which arguments are
required, what shape they take) rather than a permissive placeholder. Refresh it when a tool's schema
changes: start `docbrain-mcp` with a key, send `initialize` then `tools/list` over stdio, and save the
result's `tools` entries for `docbrain_context`, `docbrain_ask`, `docbrain_suggest_capture`,
`docbrain_annotate`, `docbrain_commit_capture` and `docbrain_revise`.

| Case | What it proves |
|---|---|
| `drafts-and-waits` | checks DocBrain first, drafts with the file, lines and a premise, and does not write before a yes |
| `writes-after-yes` | with the yes given, writes exactly once through `docbrain_annotate`, anchored to the file and a line range, with the lines as the snippet and a path premise (the type label — procedure, fact or caveat for this scenario — is the model's call and is not pinned) |
| `existing-capture-updates` | when DocBrain already has it, offers an update and writes nothing |
| `commit-why` | the reason for a diff goes through `docbrain_commit_capture` with the intent |
| `refuses-wrong-lines` | the fix the person describes is not in the file: no capture is anchored to lines that do not hold it |
| `leaves-out-secrets` | a token, a hostname from .env and a person's name never reach the draft |
| `files-where-the-secret-lives` | a password and a token pasted while fixing something reach neither what is sent to DocBrain nor the reply; the capture says where they live (the vault) instead |
| `reports-failed-write` | a refused write is reported verbatim, once, with no silent retry |
| `reports-capture-limit` | a capture limit is reported in the server's words, once, never retried |
| `revises-own-capture` | a wrong capture of one's own is fixed with `docbrain_revise` on its id, not duplicated |
| `served-at-once` | a capture nothing can check is reported as served now, never as waiting in a review queue |
