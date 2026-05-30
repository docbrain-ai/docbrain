# MCP shadow run

## Purpose

Capture a baseline of DocBrain answer quality BEFORE enabling the MCP
tool platform, then re-run AFTER, and confirm pass count ≥ baseline.
This is the gate the retirement commit (removing the legacy enricher
path) waits on.

## Prerequisites

- [ ] All 8 TODO cases authored in `tests/qa-judges/golden/management-questions-seed.yaml`.
- [ ] Judge calibration suite passing locally: `RUN_JUDGE_CALIBRATION=1 cargo test -p docbrain-core --test qa_judge_calibration -- --nocapture`. If calibration fails, do NOT proceed — judge has drifted.
- [ ] Your DocBrain cluster is running and healthy.
- [ ] Admin API key for the cluster is available locally.

## Phase 0 — Cluster prep (one-time, before Phase 1)

In your private values-overlay repo:

1. Add the `mcpTools` block to your `values-overlay.yaml`. Place it
   after the `sources:` block ends, near the `# -- GitHub OAuth` section.
   Use this template:

   ```yaml
   # -- MCP tool platform
   # Enables answer-time tool dispatch through external MCP servers.
   # Shadow-run config: both new orchestrator AND legacy JiraEnricher run
   # in parallel until the retirement commit deletes the legacy path. Pass
   # count from the judge framework must be ≥ baseline before retirement
   # can proceed.
   mcpTools:
     enabled: false   # Phase 1 capture happens with this OFF

     # OAuth: Atlassian credentials reuse the existing cluster secret
     # for the OAuth dance (per-user grants). Service-account auth picks up
     # JIRA_API_TOKEN that the existing JiraEnricher already uses — same
     # env var name, no double-config.
     oauth:
       atlassian:
         # clientId / clientSecret come from the externally managed
         # cluster secret as ATLASSIAN_OAUTH_CLIENT_ID /
         # ATLASSIAN_OAUTH_CLIENT_SECRET. Leave blank here — the
         # secret.yaml template guards against rendering empty values.
         clientId: ""
         clientSecret: ""

     serviceAccount:
       jira:
         # JIRA_API_TOKEN + JIRA_CLOUD_ID come from the cluster secret.
         apiToken: ""
         cloudId: ""

     # 256-bit master key for at-rest token encryption (mcp_oauth_tokens
     # table). Must be set in the cluster secret as MCP_OAUTH_ENCRYPTION_KEY.
     # Generate with: openssl rand -hex 32
     encryptionKey: ""
   ```

   Note: if your overlay has `existingSecret` set, helm SKIPS rendering the
   secret template entirely. The empty strings above are intentional —
   actual values live in the externally managed cluster secret (added
   manually in Phase 2 Step 1).

2. Commit in your private values-overlay repo (NOT the DocBrain repo):

   ```bash
   git add helm/docbrain/values-overlay.yaml
   git commit -m "feat: mcpTools block for shadow run"
   ```

After Phase 1 baseline is captured, flip `enabled: false` → `enabled: true`
and helm upgrade for Phase 2.

## Phase 1 — Capture baseline (pre-MCP)

State at this point: `mcpTools.enabled=false` in the overlay. The legacy
`JiraEnricher` is the only live-tool path.

1. Confirm the cluster is on the pre-MCP config:

   ```bash
   kubectl get configmap docbrain-config -n your-org \
     -o jsonpath='{.data.MCP_TOOLS_ENABLED}'
   # Expected: "false" or empty (rendered when mcpTools.enabled=false)
   ```

2. Run the seed batch against the live cluster:

   ```bash
   RUN_SEED_BATCH=1 \
     DOCBRAIN_SERVER_URL=https://docbrain.your-domain.example \
     DOCBRAIN_API_KEY=<admin-key-here> \
     cargo test -p docbrain-core --test qa_judge_seed_batch -- --nocapture
   ```

   Output artifact: `tests/qa-judges/runs/seed-batch-<timestamp>.json`.

3. Inspect the artifact:

   ```bash
   jq '.results | group_by(.verdict) | map({verdict: .[0].verdict, count: length})' \
     tests/qa-judges/runs/seed-batch-*.json | tail -20
   ```

   Record the (pass, partial, fail) counts. This is your baseline.

4. Promote the run to the canonical baseline file:

   ```bash
   cp tests/qa-judges/runs/seed-batch-<latest>.json \
      tests/qa-judges/runs/baseline-pre-mcp.json
   git add tests/qa-judges/runs/baseline-pre-mcp.json
   git commit -m "docs(qa-judges): baseline-pre-mcp captured

   N cases graded against the cluster with mcpTools.enabled=false
   (legacy JiraEnricher path only). Baseline:
     pass: X
     partial: Y
     fail: Z

   The retirement commit gate: post-flip pass count must
   be ≥ X. Captured at: <ISO-8601 timestamp>."
   ```

   (The negation rule in `.gitignore` lets this specific file be tracked.)

### Acceptance criteria for Phase 1

- [ ] Artifact file exists in `tests/qa-judges/runs/`.
- [ ] `baseline-pre-mcp.json` exists at repo root and is committed.
- [ ] Pass / partial / fail counts are recorded in the commit message.

## Phase 2 — Flip the switch (post-MCP)

State change: enable the MCP platform.

1. **Add the new env vars to the cluster secret** (manual one-time setup
   — externally managed secret, not rendered by helm because
   `existingSecret` is set):

   ```bash
   # Generate the encryption key (256-bit hex)
   openssl rand -hex 32 > /tmp/mcp-key

   # Fetch the existing secret, patch in the new keys.
   # Replace placeholders with real values from Atlassian admin console
   # and the /accessible-resources API call.
   kubectl get secret docbrain-secret -n your-org -o json \
     | jq --arg k "$(cat /tmp/mcp-key | base64)" \
          --arg cid "$(printf '%s' '<atlassian-client-id>' | base64)" \
          --arg csec "$(printf '%s' '<atlassian-client-secret>' | base64)" \
          --arg cloud "$(printf '%s' '<jira-cloud-id>' | base64)" \
       '.data["MCP_OAUTH_ENCRYPTION_KEY"]=$k
        | .data["ATLASSIAN_OAUTH_CLIENT_ID"]=$cid
        | .data["ATLASSIAN_OAUTH_CLIENT_SECRET"]=$csec
        | .data["JIRA_CLOUD_ID"]=$cloud' \
     | kubectl apply -f -

   # Verify the keys are in place (do NOT print values):
   kubectl get secret docbrain-secret -n your-org \
     -o jsonpath='{.data}' | jq 'keys[]' | grep -E 'MCP_|ATLASSIAN_|JIRA_'

   # Wipe the temp file:
   shred -u /tmp/mcp-key 2>/dev/null || rm -f /tmp/mcp-key
   ```

   Existing `JIRA_API_TOKEN` in the secret is reused — no action needed.

2. Flip the helm value. In your overlay's `values-overlay.yaml`,
   change `mcpTools.enabled: false` → `mcpTools.enabled: true`.

3. Roll out the helm change. From your private values-overlay repo:

   ```bash
   helm upgrade docbrain helm/docbrain \
     -f helm/docbrain/values.yaml \
     -f helm/docbrain/values-overlay.yaml \
     --namespace your-org
   kubectl rollout status deploy/docbrain-server \
     --namespace your-org --timeout=5m
   ```

4. Verify the orchestrator is wired:

   ```bash
   kubectl logs deploy/docbrain-server --namespace your-org --since=2m \
     | grep -i "mcp\|orchestrator\|manifest"
   # Expected log lines:
   #   "MCP_TOOLS_ENABLED=true; constructing orchestrator..."
   #   "Loaded 1 MCP manifest(s) from /etc/docbrain/mcp-manifests"
   #   "MCP orchestrator: enabled"
   ```

5. Run the seed batch AGAIN against the same cluster:

   ```bash
   RUN_SEED_BATCH=1 \
     DOCBRAIN_SERVER_URL=https://docbrain.your-domain.example \
     DOCBRAIN_API_KEY=<admin-key-here> \
     cargo test -p docbrain-core --test qa_judge_seed_batch -- --nocapture
   ```

6. Compare the new artifact against baseline:

   ```bash
   bash scripts/compare-judge-runs.sh \
     tests/qa-judges/runs/baseline-pre-mcp.json \
     tests/qa-judges/runs/seed-batch-<new>.json
   ```

### Acceptance criteria for Phase 2

- [ ] Secret contains `MCP_OAUTH_ENCRYPTION_KEY`, `ATLASSIAN_OAUTH_CLIENT_ID`,
      `ATLASSIAN_OAUTH_CLIENT_SECRET`, `JIRA_CLOUD_ID`.
- [ ] Helm upgrade succeeds; rollout completes within 5m.
- [ ] Server logs show all three orchestrator construction lines.
- [ ] New seed-batch artifact exists in `tests/qa-judges/runs/`.

## Phase 3 — Gate decision

Run the comparison script and act on the exit code:

- **Exit 0 (PASS count NEW ≥ baseline AND no PASS → FAIL regression)**:
  Proceed to the retirement commit. The retirement is safe.

- **Exit 2 (a question regressed PASS → FAIL)**:
  Block the retirement commit. The new platform is producing a wrong answer
  in a case the old platform handled. Either the tool is returning bad data,
  the dispatch decision is wrong, or the block formatter is corrupting
  the prompt.

- **Exit 3 (PASS count NEW < baseline, but no per-case regression)**:
  Investigate. Common causes:
  - Orchestrator is double-fetching and getting rate-limited.
  - `mcpTools.enabled` not propagated to all pod replicas (rolling update
    incomplete).
  - Atlassian credentials not fully populated.
  - JiraEnricher's hardcoded `=== Live Jira status ===` block is
    conflicting with orchestrator's `=== Live tool: ===` block in the
    synthesis prompt. (Both should be present during shadow; they
    shouldn't conflict — check the prompt-capture seam.)

### Acceptance criteria for Phase 3

- [ ] `compare-judge-runs.sh` exits 0.
- [ ] Decision (proceed / investigate / block) recorded in the LESSONS.md
      decision journal entry for the shadow run.

## Rollback (if anything goes wrong)

```bash
# Revert to pre-MCP config without code rollback:
helm upgrade docbrain helm/docbrain \
  -f helm/docbrain/values.yaml \
  -f helm/docbrain/values-overlay.yaml \
  --set mcpTools.enabled=false \
  --namespace your-org
```

This disables the orchestrator at the config layer — code stays deployed
but the runtime path skips the orchestrator entirely (the runtime
gating invariant: when the flag is off, no orchestrator is constructed).
