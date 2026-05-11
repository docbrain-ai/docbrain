#!/usr/bin/env bash
# Compare two judge-run artifacts and report deltas.
# Usage: bash scripts/compare-judge-runs.sh <baseline.json> <new.json>
#
# Exit codes:
#   0 — pass count new >= baseline AND no per-case PASS -> FAIL regression.
#   1 — usage / I/O error.
#   2 — at least one case regressed from pass -> fail.
#   3 — aggregate pass count dropped (no per-case regression, but bucket shifted).

set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "Usage: $0 <baseline.json> <new.json>"
  exit 1
fi

baseline=$1
new=$2

if [[ ! -f $baseline || ! -f $new ]]; then
  echo "FAIL: one or both files missing"
  exit 1
fi

bl_pass=$(jq '[.results[] | select(.verdict == "pass")] | length' "$baseline")
bl_partial=$(jq '[.results[] | select(.verdict == "partial")] | length' "$baseline")
bl_fail=$(jq '[.results[] | select(.verdict == "fail")] | length' "$baseline")

nw_pass=$(jq '[.results[] | select(.verdict == "pass")] | length' "$new")
nw_partial=$(jq '[.results[] | select(.verdict == "partial")] | length' "$new")
nw_fail=$(jq '[.results[] | select(.verdict == "fail")] | length' "$new")

echo "Verdict counts:"
echo "                  pass  partial  fail"
printf "  baseline:        %3d      %3d   %3d\n" "$bl_pass" "$bl_partial" "$bl_fail"
printf "  new:             %3d      %3d   %3d\n" "$nw_pass" "$nw_partial" "$nw_fail"
echo

# Per-case regression check: any case that was pass in baseline and is fail in new.
regressed=$(jq -n --slurpfile b "$baseline" --slurpfile n "$new" '
  $b[0].results as $bcases
  | $n[0].results as $ncases
  | [
      $bcases[] as $bc
      | $ncases[] | select(.id == $bc.id and .verdict == "fail" and $bc.verdict == "pass")
      | {id, baseline_verdict: $bc.verdict, new_verdict: .verdict, new_reason: (.reason // "")}
    ]
')

regressed_count=$(echo "$regressed" | jq 'length')

if [[ $regressed_count -gt 0 ]]; then
  echo "FAIL: $regressed_count case(s) regressed PASS -> FAIL:"
  echo "$regressed" | jq -r '.[] | "  - \(.id): \(.new_reason)"'
  exit 2
fi

if [[ $nw_pass -lt $bl_pass ]]; then
  echo "FAIL: pass count dropped ($bl_pass -> $nw_pass)"
  exit 3
fi

echo "PASS: $nw_pass >= $bl_pass, no regressions."
