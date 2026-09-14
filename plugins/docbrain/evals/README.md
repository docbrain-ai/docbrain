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

| Case | What it proves |
|---|---|
| `drafts-and-waits` | checks DocBrain first, drafts with the file, lines and a premise, and does not write before a yes |
| `writes-after-yes` | with the yes given, writes exactly once through `docbrain_annotate` with the type, the anchor and a premise |
| `existing-capture-updates` | when DocBrain already has it, offers an update and writes nothing |
| `commit-why` | the reason for a diff goes through `docbrain_commit_capture` with the intent |
| `refuses-wrong-lines` | the fix the person describes is not in the file: no capture is anchored to lines that do not hold it |
| `leaves-out-secrets` | a token, a hostname from .env and a person's name never reach the draft |
| `reports-failed-write` | a refused write is reported verbatim, once, with no silent retry |
