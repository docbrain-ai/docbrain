# QA judges

LLM-based answer graders for non-deterministic DocBrain answers.

## When to use

- Live-tool answers (Jira status, GitHub issue state, etc.) where the
  expected content shifts over time. Substring evals (`crates/docbrain-core/src/eval.rs`)
  are wrong here — they'd flag a correct answer as "missing string X" when X
  is `"In Progress"` but today's true state is `"Done"`.
- Shadow-run grading: batch of management questions graded
  against rubrics rather than exact strings.

## Files

- `judge-system-prompt.md` — the verbatim system prompt sent to the judge.
- `calibration.yaml` — 5 fixed tuples; must pass before any production batch.
- `golden/` — authored seed batches. One YAML file per batch.

## Running

```bash
RUN_JUDGE_CALIBRATION=1 cargo test -p docbrain-core --test qa_judge_calibration -- --nocapture
```

Requires the same LLM env config the server uses (e.g. `ANTHROPIC_API_KEY`,
provider/model selection in `config/default.yaml`).

## Seed batch — management questions

The v1 ship-gate batch lives at:

```
tests/qa-judges/golden/management-questions-seed.yaml
```

Two cases ship scaffolded (helm_migration_blocker, aitoolintg_35_status).
The remaining 8 are TODO markers — the operator authors them against their
operational reality (Jira tickets, deployment status, ownership questions
specific to your org).

Run the batch end-to-end against a live DocBrain server:

```bash
RUN_SEED_BATCH=1 \
  DOCBRAIN_SERVER_URL=http://localhost:3000 \
  DOCBRAIN_API_KEY=$(grep BOOTSTRAP_ADMIN_KEY .env | cut -d= -f2) \
  cargo test -p docbrain-core --test qa_judge_seed_batch -- --nocapture
```

Output: `tests/qa-judges/runs/seed-batch-<timestamp>.json` with per-case
verdicts. Compare against `tests/qa-judges/runs/baseline-pre-mcp.json`
(captured before flipping mcp_tools.enabled).

## Adding a new batch

1. Author `tests/qa-judges/golden/<name>.yaml` with `question`, `expected_facts`,
   `forbidden_claims` fields per case.
2. Write an integration test that loads the batch + iterates `judge_one()`.
3. CI gate: PASS count >= baseline. Record baseline in
   `tests/qa-judges/runs/baseline-<batch>.json`.

## When calibration fails

DO NOT run production batches. The judge has drifted — either:
- The judge model upgraded silently (check `DEFAULT_JUDGE_MODEL` const).
- The system prompt was edited without re-validating.
- The provider changed behavior.

Investigate before relying on judge verdicts again.
