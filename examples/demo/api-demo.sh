#!/usr/bin/env bash
set -euo pipefail
# ═══════════════════════════════════════════════════════════════════════════
# DocBrain API Demo — Terminal Commands for Video Recording
#
# Run these commands one at a time during the video to demonstrate
# the API capabilities. Each section corresponds to a video scene.
#
# Prerequisites:
#   export DOCBRAIN_API_KEY="db_sk_..."
#   export DOCBRAIN_API_URL="http://localhost:3000"
# ═══════════════════════════════════════════════════════════════════════════

API="${DOCBRAIN_API_URL:-http://localhost:3000}"
KEY="${DOCBRAIN_API_KEY:?Set DOCBRAIN_API_KEY}"
AUTH="Authorization: Bearer $KEY"
CT="Content-Type: application/json"

BOLD='\033[1m'
NC='\033[0m'

section() {
  echo ""
  echo -e "${BOLD}═══ $1 ═══${NC}"
  echo ""
}

# ── Health Check ──────────────────────────────────────────────────────────────
section "1. Health Check"
echo '$ curl http://localhost:3000/api/v1/health'
curl -s "$API/api/v1/health" | python3 -m json.tool
echo ""

# ── Server Config ─────────────────────────────────────────────────────────────
section "2. Server Configuration"
echo '$ curl http://localhost:3000/api/v1/config'
curl -s "$API/api/v1/config" | python3 -m json.tool
echo ""

# ── Ask a Question ────────────────────────────────────────────────────────────
section "3. Ask: How do I deploy to production?"
curl -s -X POST "$API/api/v1/ask" \
  -H "$AUTH" -H "$CT" \
  -d '{"question": "How do I deploy to production?"}' | python3 -m json.tool
echo ""

# ── Ask Follow-Up (Working Memory) ───────────────────────────────────────────
section "4. Follow-Up: What about rollback?"
echo "(Using same session_id for multi-turn)"
# Get session_id from previous response in a real scenario
curl -s -X POST "$API/api/v1/ask" \
  -H "$AUTH" -H "$CT" \
  -d '{"question": "What about rolling back if something goes wrong?"}' | python3 -m json.tool
echo ""

# ── CI/CD Capture ─────────────────────────────────────────────────────────────
section "5. CI Capture: Analyze a Merged PR"
curl -s -X POST "$API/api/v1/ci/analyze" \
  -H "$AUTH" -H "$CT" \
  -d '{
    "pr_number": 3001,
    "repo": "acme/platform",
    "pr_title": "Add request tracing to search service",
    "pr_body": "Implements OpenTelemetry distributed tracing for the search service. Propagates trace context via W3C traceparent header. Adds custom spans for each search pipeline stage (parse, embed, retrieve, rerank, generate).",
    "diff_stat": "+245 -18",
    "changed_files": "src/search/tracing.rs,src/search/pipeline.rs",
    "labels": "observability",
    "author": "sarah.chen@acme.com"
  }' | python3 -m json.tool
echo ""

# ── Create a Fragment ─────────────────────────────────────────────────────────
section "6. Create a Knowledge Fragment"
curl -s -X POST "$API/api/v1/fragments" \
  -H "$AUTH" -H "$CT" \
  -d '{
    "fragment_type": "caveat",
    "summary": "OpenSearch bulk indexing fails silently when a single document has a mapping conflict",
    "content": "When using the bulk API, if one document fails due to a mapping conflict, the entire batch reports success (HTTP 200) but the individual item has an error in the response body. Always check the errors field in the bulk response. We lost 2 hours debugging this during the search migration.",
    "source_type": "conversation_distill",
    "source_ref": "slack://C01234/thread-5678",
    "confidence": 0.85,
    "space": "PLATFORM"
  }' | python3 -m json.tool
echo ""

# ── Lint Content ──────────────────────────────────────────────────────────────
section "7. Lint: Check Content Against Style Rules"
curl -s -X POST "$API/api/v1/quality/lint" \
  -H "$AUTH" -H "$CT" \
  -d '{
    "content": "This is a simple guide to deploying services. Just follow these easy steps and you will obviously have your service running.\n\nVisit https://internal.acme-platform.com/deploy for more details."
  }' | python3 -m json.tool
echo ""

# ── Freshness Report ─────────────────────────────────────────────────────────
section "8. Freshness Report"
curl -s "$API/api/v1/freshness" \
  -H "$AUTH" | python3 -m json.tool
echo ""

# ── Autopilot Summary ────────────────────────────────────────────────────────
section "9. Autopilot Summary"
curl -s "$API/api/v1/autopilot/summary" \
  -H "$AUTH" | python3 -m json.tool
echo ""

# ── Quality Report ────────────────────────────────────────────────────────────
section "10. Quality Report"
curl -s "$API/api/v1/quality/report" \
  -H "$AUTH" | python3 -m json.tool
echo ""

# ── Analytics ─────────────────────────────────────────────────────────────────
section "11. Analytics (30-day)"
curl -s "$API/api/v1/analytics?days=30" \
  -H "$AUTH" | python3 -m json.tool
echo ""

# ── Connector Health ──────────────────────────────────────────────────────────
section "12. Wiki Connector Health"
curl -s http://localhost:4000/health | python3 -m json.tool
echo ""

# ── Events ────────────────────────────────────────────────────────────────────
section "13. Recent Events"
curl -s "$API/api/v1/events?limit=5" \
  -H "$AUTH" | python3 -m json.tool
echo ""

section "Demo Complete!"
echo "See VIDEO_WALKTHROUGH.md for the full video script."
