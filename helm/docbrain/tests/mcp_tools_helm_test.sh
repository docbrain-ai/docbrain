#!/usr/bin/env bash
# Smoke-test the MCP tools helm wiring.
#
# Asserts:
#   1. mcpTools.enabled=false renders no MCP configmap, no volume, no env.
#   2. mcpTools.enabled=true renders configmap + volume + env + jira manifest
#      content with the offline_access scope intact.
#
# Uses `helm template` (offline rendering, no cluster needed). If `helm` is
# not installed, the script reports and exits 0 so CI on barebones runners
# doesn't fail spuriously.
#
# Run from repo root:
#   bash helm/docbrain/tests/mcp_tools_helm_test.sh

set -euo pipefail

if ! command -v helm >/dev/null 2>&1; then
  echo "SKIP: helm not installed; cannot run helm template smoke test"
  exit 0
fi

cd "$(dirname "$0")/../../.."

CHART="helm/docbrain"

# ── Test 1: disabled mode ─────────────────────────────────────────────────
out=$(helm template "$CHART" --set mcpTools.enabled=false)
if echo "$out" | grep -q "mcp-manifests"; then
  echo "FAIL: mcp-manifests should not appear when mcpTools.enabled=false"
  exit 1
fi
if echo "$out" | grep -q "MCP_MANIFEST_DIR"; then
  echo "FAIL: MCP_MANIFEST_DIR env should not be set when disabled"
  exit 1
fi
echo "PASS: disabled mode has no MCP wiring"

# ── Test 2: enabled mode ──────────────────────────────────────────────────
out=$(helm template "$CHART" \
  --set mcpTools.enabled=true \
  --set mcpTools.encryptionKey=test-key)

echo "$out" | grep -q "kind: ConfigMap" || { echo "FAIL: no ConfigMap rendered"; exit 1; }
echo "$out" | grep -q "/etc/docbrain/mcp-manifests" || { echo "FAIL: mount path missing"; exit 1; }
echo "$out" | grep -q 'MCP_MANIFEST_DIR: "/etc/docbrain/mcp-manifests"' || { echo "FAIL: MCP_MANIFEST_DIR env not set"; exit 1; }
echo "$out" | grep -q 'MCP_TOOLS_ENABLED: "true"' || { echo "FAIL: MCP_TOOLS_ENABLED env not 'true'"; exit 1; }
echo "$out" | grep -q "id: jira" || { echo "FAIL: jira manifest content not embedded in configmap"; exit 1; }
echo "$out" | grep -q "offline_access" || { echo "FAIL: offline_access scope missing from rendered manifest"; exit 1; }
echo "$out" | grep -q "name: mcp-manifests" || { echo "FAIL: volume/volumeMount not rendered"; exit 1; }
echo "PASS: enabled mode wires configmap, volume, env, and ships jira manifest with offline_access"

echo
echo "All MCP tools helm smoke tests passed."
