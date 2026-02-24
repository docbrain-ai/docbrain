#!/usr/bin/env bash
# ============================================================================
# DocBrain Real-World Pain Point Test Suite
# ============================================================================
# Tests all 7 major documentation pain points against REAL production data.
# No mocks. No fakes. Every test hits the actual DocBrain API with real docs.
#
# Usage: ./scripts/test-real-world.sh [API_KEY]
# ============================================================================

set -euo pipefail

API_URL="${DOCBRAIN_API_URL:-http://localhost:3000}"
API_KEY="${1:-${DOCBRAIN_API_KEY:-db_sk_xgxmf8nwsm6mbsw2pftd0x47efj8x2lc}}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
BOLD='\033[1m'
NC='\033[0m'

PASS=0
FAIL=0
WARN=0
RESULTS=()

# ── Helpers ──────────────────────────────────────────────────────────────────

pass() {
  PASS=$((PASS + 1))
  RESULTS+=("${GREEN}PASS${NC} $1")
  echo -e "  ${GREEN}✓${NC} $1"
}

fail() {
  FAIL=$((FAIL + 1))
  RESULTS+=("${RED}FAIL${NC} $1 — $2")
  echo -e "  ${RED}✗${NC} $1 — $2"
}

warn() {
  WARN=$((WARN + 1))
  RESULTS+=("${YELLOW}WARN${NC} $1 — $2")
  echo -e "  ${YELLOW}⚠${NC} $1 — $2"
}

ask() {
  local question="$1"
  local response
  response=$(curl -s -w "\n%{http_code}" "$API_URL/api/v1/ask" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $API_KEY" \
    -d "{\"question\": \"$question\"}" 2>/dev/null)

  local http_code
  http_code=$(echo "$response" | tail -1)
  local body
  body=$(echo "$response" | sed '$d')

  if [ "$http_code" != "200" ]; then
    echo "ERROR:$http_code"
    return
  fi
  echo "$body"
}

# ── Pre-flight ───────────────────────────────────────────────────────────────

echo -e "\n${BOLD}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║  DocBrain Real-World Pain Point Test Suite                    ║${NC}"
echo -e "${BOLD}╚══════════════════════════════════════════════════════════════╝${NC}\n"

echo -e "${CYAN}Target:${NC} $API_URL"
echo -e "${CYAN}Date:${NC}   $(date -u '+%Y-%m-%d %H:%M:%S UTC')"
echo ""

# Check server is up
CONFIG=$(curl -s "$API_URL/api/v1/config" 2>/dev/null || echo "")
if [ -z "$CONFIG" ]; then
  echo -e "${RED}Server not reachable at $API_URL${NC}"
  exit 1
fi
echo -e "${GREEN}Server reachable.${NC} Features: $(echo "$CONFIG" | python3 -c "
import json, sys
d = json.load(sys.stdin)
enabled = [k for k, v in d.get('features', {}).items() if v]
print(f'{len(enabled)} enabled')
" 2>/dev/null || echo "unknown")"
echo ""

# ============================================================================
# PAIN POINT 1: Search Quality — "I can't find what I need"
# ============================================================================
echo -e "${BOLD}━━━ Pain Point 1: Search Quality ━━━${NC}"
echo -e "  Testing: Can users find relevant answers to real questions?\n"

# Test 1.1: Specific how-to question (should get actionable answer)
RESPONSE=$(ask "How do I configure alerting rules in Grafana?")
if echo "$RESPONSE" | grep -q "ERROR:"; then
  fail "1.1 Grafana alerting how-to" "API error: $RESPONSE"
else
  ANSWER=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer',''))" 2>/dev/null)
  SOURCES=$(echo "$RESPONSE" | python3 -c "import json,sys; print(len(json.load(sys.stdin).get('sources',[])))" 2>/dev/null)

  # Must have sources and must NOT say "I don't have enough information"
  if echo "$ANSWER" | grep -qi "don't have enough information\|cannot find\|no relevant"; then
    fail "1.1 Grafana alerting how-to" "Answer admits lack of knowledge (sources: $SOURCES)"
  elif [ "$SOURCES" -ge 2 ]; then
    pass "1.1 Grafana alerting how-to ($SOURCES sources, has actionable content)"
  else
    warn "1.1 Grafana alerting how-to" "Only $SOURCES sources returned"
  fi
fi

# Test 1.2: Troubleshooting question (should reference runbooks/incidents)
RESPONSE=$(ask "What should I do if I see ImagePullBackOff errors in Kubernetes?")
if echo "$RESPONSE" | grep -q "ERROR:"; then
  fail "1.2 ImagePullBackOff troubleshooting" "API error"
else
  ANSWER=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer',''))" 2>/dev/null)
  SOURCES=$(echo "$RESPONSE" | python3 -c "import json,sys; print(len(json.load(sys.stdin).get('sources',[])))" 2>/dev/null)

  ANSWER_LEN=${#ANSWER}
  if [ "$SOURCES" -ge 1 ] && [ "$ANSWER_LEN" -ge 100 ]; then
    # Has sources and substantive answer — pass even if answer hedges (grounding is correct)
    pass "1.2 ImagePullBackOff troubleshooting ($SOURCES sources, ${ANSWER_LEN} chars)"
  elif [ "$SOURCES" -ge 1 ]; then
    warn "1.2 ImagePullBackOff troubleshooting" "Sources found but answer too short"
  else
    fail "1.2 ImagePullBackOff troubleshooting" "No sources and no substantive answer"
  fi
fi

# Test 1.3: Architectural/explain question
RESPONSE=$(ask "What is our code review process?")
if echo "$RESPONSE" | grep -q "ERROR:"; then
  fail "1.3 Code review process" "API error"
else
  ANSWER=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer',''))" 2>/dev/null)
  SOURCES=$(echo "$RESPONSE" | python3 -c "import json,sys; print(len(json.load(sys.stdin).get('sources',[])))" 2>/dev/null)
  INTENT=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('intent',''))" 2>/dev/null)

  if echo "$ANSWER" | grep -qi "don't have enough\|cannot find"; then
    fail "1.3 Code review process" "No answer found"
  elif [ "$SOURCES" -ge 2 ]; then
    pass "1.3 Code review process (intent: $INTENT, $SOURCES sources)"
  else
    warn "1.3 Code review process" "Only $SOURCES sources"
  fi
fi

# Test 1.4: Cross-space question (should pull from multiple spaces)
RESPONSE=$(ask "How do we handle database migrations across our services?")
if echo "$RESPONSE" | grep -q "ERROR:"; then
  fail "1.4 Cross-space database migrations" "API error"
else
  ANSWER=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer',''))" 2>/dev/null)
  SOURCES=$(echo "$RESPONSE" | python3 -c "import json,sys; print(len(json.load(sys.stdin).get('sources',[])))" 2>/dev/null)

  if echo "$ANSWER" | grep -qi "don't have enough\|cannot find"; then
    fail "1.4 Cross-space database migrations" "No answer"
  elif [ "$SOURCES" -ge 2 ]; then
    pass "1.4 Cross-space database migrations ($SOURCES sources)"
  else
    warn "1.4 Cross-space database migrations" "Only $SOURCES sources"
  fi
fi

# Test 1.5: Grounding check — answer must NOT hallucinate
RESPONSE=$(ask "What is the exact command to deploy to production using our CI/CD pipeline?")
if echo "$RESPONSE" | grep -q "ERROR:"; then
  fail "1.5 Grounding — no hallucinated commands" "API error"
else
  ANSWER=$(echo "$RESPONSE" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer',''))" 2>/dev/null)

  # Check the answer doesn't contain generic kubectl/helm commands that aren't from the docs
  if echo "$ANSWER" | grep -qi "I cannot provide the exact command\|documentation doesn't specify the exact\|not enough information"; then
    pass "1.5 Grounding — correctly admits when info not found"
  elif echo "$ANSWER" | grep -qi "Source:\|based on.*documentation\|according to"; then
    pass "1.5 Grounding — cites sources for specific commands"
  else
    warn "1.5 Grounding check" "Answer may contain ungrounded content"
  fi
fi

echo ""

# ============================================================================
# PAIN POINT 2: Stale Documentation Detection
# ============================================================================
echo -e "${BOLD}━━━ Pain Point 2: Stale Documentation Detection ━━━${NC}"
echo -e "  Testing: Does freshness scoring identify outdated docs?\n"

# Pipe freshness response through python3 to extract summary (avoids huge shell var for 12K docs)
FRESHNESS_PARSED=$(curl -s "$API_URL/api/v1/freshness" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null | python3 -c "
import json, sys
try:
    d = json.load(sys.stdin)
    s = d.get('summary', {})
    docs = d.get('documents', [])
    doc0_signals = 0
    if docs:
        signals = ['time_decay_score', 'engagement_score', 'content_currency_score', 'link_health_score', 'contradiction_score']
        doc0_signals = sum(1 for sg in signals if sg in docs[0])
    print(f\"{s.get('total_docs',0)}|{s.get('fresh',0)}|{s.get('stale',0)}|{s.get('outdated',0)}|{round(s.get('avg_score',0),1)}|{doc0_signals}\")
except:
    print('0|0|0|0|0|0')
" 2>/dev/null || echo "0|0|0|0|0|0")

IFS='|' read -r TOTAL FRESH STALE OUTDATED AVG DOC_SIGNALS <<< "$FRESHNESS_PARSED"

if [ "$TOTAL" -ge 100 ]; then
  pass "2.1 Freshness coverage ($TOTAL docs scored)"
else
  fail "2.1 Freshness coverage" "Only $TOTAL docs scored"
fi

if [ "$FRESH" -gt 0 ] && [ "$STALE" -gt 0 ]; then
  pass "2.2 Score differentiation (fresh=$FRESH, stale=$STALE, outdated=$OUTDATED, avg=$AVG)"
else
  fail "2.2 Score differentiation" "All docs in same bucket (fresh=$FRESH, stale=$STALE)"
fi

if [ "$DOC_SIGNALS" = "5" ]; then
  pass "2.3 All 5 freshness signals present ($DOC_SIGNALS/5)"
else
  fail "2.3 Freshness signals" "Only $DOC_SIGNALS/5 signals present"
fi

# 2.4: Per-space freshness
FRESHNESS_SPACE=$(curl -s "$API_URL/api/v1/freshness?space=SAAS" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null)
if echo "$FRESHNESS_SPACE" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['summary']['total_docs'] > 0" 2>/dev/null; then
  SPACE_TOTAL=$(echo "$FRESHNESS_SPACE" | python3 -c "import json,sys; print(json.load(sys.stdin)['summary']['total_docs'])" 2>/dev/null)
  pass "2.4 Per-space freshness works (SAAS: $SPACE_TOTAL docs)"
else
  fail "2.4 Per-space freshness" "Empty or error for SAAS space"
fi

echo ""

# ============================================================================
# PAIN POINT 3: New Employee Onboarding — "Where do I even start?"
# ============================================================================
echo -e "${BOLD}━━━ Pain Point 3: New Employee Onboarding ━━━${NC}"
echo -e "  Testing: Does onboarding surface week-1 orientation docs, not deep specs?\n"

for SPACE in SAAS ENG; do
  ONBOARD=$(curl -s "$API_URL/api/v1/onboard/$SPACE" \
    -H "Authorization: Bearer $API_KEY" 2>/dev/null)

  if [ -z "$ONBOARD" ]; then
    fail "3.1 Onboarding API ($SPACE)" "Empty response"
    continue
  fi

  DOC_COUNT=$(echo "$ONBOARD" | python3 -c "import json,sys; d=json.load(sys.stdin); print(len(d.get('reading_list', d.get('reading_list', d.get('recommended_reading', [])))))" 2>/dev/null || echo "0")

  # 3.1: Should return 5-12 docs
  if [ "$DOC_COUNT" -ge 5 ] && [ "$DOC_COUNT" -le 15 ]; then
    pass "3.1 Onboarding doc count ($SPACE: $DOC_COUNT docs)"
  elif [ "$DOC_COUNT" -gt 0 ]; then
    warn "3.1 Onboarding doc count ($SPACE)" "$DOC_COUNT docs (expected 5-12)"
  else
    fail "3.1 Onboarding doc count ($SPACE)" "No docs returned"
    continue
  fi

  # 3.2: Should NOT contain deep infrastructure docs (HLDs, LLDs, runbooks for specific services)
  BAD_DOCS=$(echo "$ONBOARD" | python3 -c "
import json, sys
d = json.load(sys.stdin)
docs = d.get('reading_list', d.get('recommended_reading', []))
bad_keywords = ['autoscaling', 'terraform', 'helm migration', 'mTLS', 'grafana alert',
                'database migration', 'rollback', 'canary deploy', 'EKS', 'infrastructure']
bad = []
for doc in docs:
    title = doc.get('title', '').lower()
    for kw in bad_keywords:
        if kw.lower() in title:
            bad.append(doc.get('title', ''))
            break
print(len(bad))
" 2>/dev/null || echo "0")

  if [ "$BAD_DOCS" = "0" ]; then
    pass "3.2 No deep-spec docs in onboarding ($SPACE)"
  else
    warn "3.2 Onboarding quality ($SPACE)" "$BAD_DOCS deep-spec docs found in results"
  fi

  # 3.3: Each doc should have a reason
  HAS_REASONS=$(echo "$ONBOARD" | python3 -c "
import json, sys
d = json.load(sys.stdin)
docs = d.get('reading_list', d.get('recommended_reading', []))
with_reason = sum(1 for doc in docs if doc.get('reason') or doc.get('why'))
print(f'{with_reason}/{len(docs)}')
" 2>/dev/null || echo "0/0")

  if echo "$HAS_REASONS" | grep -q "^[0-9]*/[0-9]*$" && [ "$(echo "$HAS_REASONS" | cut -d/ -f1)" = "$(echo "$HAS_REASONS" | cut -d/ -f2)" ]; then
    pass "3.3 All onboarding docs have reasons ($SPACE: $HAS_REASONS)"
  else
    warn "3.3 Onboarding reasons ($SPACE)" "Only $HAS_REASONS docs have reasons"
  fi
done

echo ""

# ============================================================================
# PAIN POINT 4: Conversational Memory — "It forgot what I just asked"
# ============================================================================
echo -e "${BOLD}━━━ Pain Point 4: Conversational Memory ━━━${NC}"
echo -e "  Testing: Multi-turn follow-ups maintain context\n"

# Turn 1: Establish context
RESPONSE1=$(curl -s "$API_URL/api/v1/ask" \
  -H "Content-Type: application/json" \
  -H "Authorization: Bearer $API_KEY" \
  -d '{"question": "What deployment tools do we use?"}' 2>/dev/null)

SESSION_ID=$(echo "$RESPONSE1" | python3 -c "import json,sys; print(json.load(sys.stdin).get('session_id',''))" 2>/dev/null)
ANSWER1=$(echo "$RESPONSE1" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer','')[:100])" 2>/dev/null)

if [ -n "$SESSION_ID" ] && [ "$SESSION_ID" != "null" ] && [ "$SESSION_ID" != "" ]; then
  pass "4.1 Session created (id: ${SESSION_ID:0:8}...)"

  # Turn 2: Follow-up using session
  RESPONSE2=$(curl -s "$API_URL/api/v1/ask" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $API_KEY" \
    -d "{\"question\": \"How do I rollback if something goes wrong with those?\", \"session_id\": \"$SESSION_ID\"}" 2>/dev/null)

  ANSWER2=$(echo "$RESPONSE2" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer',''))" 2>/dev/null)
  TURN=$(echo "$RESPONSE2" | python3 -c "import json,sys; print(json.load(sys.stdin).get('turn', 0))" 2>/dev/null)

  # The follow-up answer should reference deployment context (not be confused)
  if [ "$TURN" -ge 2 ] || echo "$ANSWER2" | grep -qi "deploy\|rollback\|revert\|previous version"; then
    pass "4.2 Follow-up maintains context (turn: $TURN)"
  else
    fail "4.2 Follow-up context" "Answer doesn't reference prior deployment context"
  fi

  # Turn 3: Another follow-up
  RESPONSE3=$(curl -s "$API_URL/api/v1/ask" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $API_KEY" \
    -d "{\"question\": \"What about monitoring after that?\", \"session_id\": \"$SESSION_ID\"}" 2>/dev/null)

  ANSWER3=$(echo "$RESPONSE3" | python3 -c "import json,sys; print(json.load(sys.stdin).get('answer',''))" 2>/dev/null)

  if echo "$ANSWER3" | grep -qi "monitor\|observ\|alert\|metric\|dashboard\|grafana"; then
    pass "4.3 Third turn maintains conversation thread"
  else
    warn "4.3 Third turn context" "May have lost conversation thread"
  fi
else
  fail "4.1 Session creation" "No session_id returned"
  fail "4.2 Follow-up context" "Skipped (no session)"
  fail "4.3 Third turn" "Skipped (no session)"
fi

echo ""

# ============================================================================
# PAIN POINT 5: Gap Analysis — "What docs are we missing?"
# ============================================================================
echo -e "${BOLD}━━━ Pain Point 5: Documentation Gap Analysis ━━━${NC}"
echo -e "  Testing: Autopilot identifies real documentation gaps\n"

GAPS=$(curl -s "$API_URL/api/v1/autopilot/gaps" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null)

GAP_VALID=$(echo "$GAPS" | python3 -c "import json,sys; d=json.load(sys.stdin); print('ok' if isinstance(d, list) else 'err')" 2>/dev/null || echo "err")
if [ "$GAP_VALID" != "ok" ]; then
  fail "5.1 Gap API availability" "API error or invalid response"
else
  GAP_COUNT=$(echo "$GAPS" | python3 -c "import json,sys; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "0")

  # 5.1: Should have identified some gaps (we have 17 thumbs-down episodes)
  if [ "$GAP_COUNT" -ge 5 ]; then
    pass "5.1 Gap detection ($GAP_COUNT gaps found)"
  elif [ "$GAP_COUNT" -gt 0 ]; then
    warn "5.1 Gap detection" "Only $GAP_COUNT gaps (expected 5+ from feedback data)"
  else
    fail "5.1 Gap detection" "No gaps found despite negative feedback data"
  fi

  # 5.2: No duplicate labels
  DUPES=$(echo "$GAPS" | python3 -c "
import json, sys
from collections import Counter
gaps = json.load(sys.stdin)
labels = [g['label'].lower().strip() for g in gaps]
dupes = {k: v for k, v in Counter(labels).items() if v > 1}
print(len(dupes))
" 2>/dev/null || echo "0")

  if [ "$DUPES" = "0" ]; then
    pass "5.2 No exact-duplicate gap labels"
  else
    fail "5.2 Duplicate gaps" "$DUPES duplicate labels found"
  fi

  # 5.3: Near-duplicate check (fuzzy)
  NEAR_DUPES=$(echo "$GAPS" | python3 -c "
import json, sys
from difflib import SequenceMatcher
gaps = json.load(sys.stdin)
labels = [g['label'] for g in gaps]
near = []
for i in range(len(labels)):
    for j in range(i+1, len(labels)):
        ratio = SequenceMatcher(None, labels[i].lower(), labels[j].lower()).ratio()
        if ratio > 0.75:
            near.append(f'{labels[i]} ≈ {labels[j]} ({ratio:.0%})')
if near:
    for n in near[:5]:
        print(n)
    print(f'TOTAL:{len(near)}')
else:
    print('TOTAL:0')
" 2>/dev/null)

  NEAR_COUNT=$(echo "$NEAR_DUPES" | grep "TOTAL:" | cut -d: -f2)
  if [ "$NEAR_COUNT" = "0" ]; then
    pass "5.3 No near-duplicate gap labels"
  else
    fail "5.3 Near-duplicate gaps" "$NEAR_COUNT near-duplicates found"
    echo "$NEAR_DUPES" | grep -v "TOTAL:" | head -3 | while read -r line; do
      echo -e "        ${YELLOW}→ $line${NC}"
    done
  fi

  # 5.4: Gap labels should be descriptive (not "Uncategorized")
  UNCATEGORIZED=$(echo "$GAPS" | python3 -c "
import json, sys
gaps = json.load(sys.stdin)
bad = [g['label'] for g in gaps if 'uncategorized' in g['label'].lower() or 'unknown' in g['label'].lower()]
print(len(bad))
" 2>/dev/null || echo "0")

  if [ "$UNCATEGORIZED" = "0" ]; then
    pass "5.4 All gaps have descriptive labels"
  else
    fail "5.4 Gap label quality" "$UNCATEGORIZED gaps have generic labels"
  fi

  # 5.5: Gaps should have severity
  WITH_SEVERITY=$(echo "$GAPS" | python3 -c "
import json, sys
gaps = json.load(sys.stdin)
valid = sum(1 for g in gaps if g.get('severity') in ['critical','high','medium','low'])
print(f'{valid}/{len(gaps)}')
" 2>/dev/null || echo "0/0")

  SEV_NUM=$(echo "$WITH_SEVERITY" | cut -d/ -f1)
  SEV_TOTAL=$(echo "$WITH_SEVERITY" | cut -d/ -f2)
  if [ "$SEV_NUM" = "$SEV_TOTAL" ] && [ "$SEV_TOTAL" != "0" ]; then
    pass "5.5 All gaps have valid severity ($WITH_SEVERITY)"
  else
    fail "5.5 Gap severity" "Only $WITH_SEVERITY have valid severity"
  fi
fi

# 5.6: Summary endpoint
SUMMARY=$(curl -s "$API_URL/api/v1/autopilot/summary" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null)

if echo "$SUMMARY" | python3 -c "import json,sys; d=json.load(sys.stdin); assert 'total_gaps' in d" 2>/dev/null; then
  TOTAL_GAPS=$(echo "$SUMMARY" | python3 -c "import json,sys; print(json.load(sys.stdin)['total_gaps'])" 2>/dev/null)
  OPEN_GAPS=$(echo "$SUMMARY" | python3 -c "import json,sys; print(json.load(sys.stdin)['open_gaps'])" 2>/dev/null)
  pass "5.6 Autopilot summary works (total=$TOTAL_GAPS, open=$OPEN_GAPS)"
else
  fail "5.6 Autopilot summary" "API error"
fi

echo ""

# ============================================================================
# PAIN POINT 6: Draft Quality — "AI writes generic garbage"
# ============================================================================
echo -e "${BOLD}━━━ Pain Point 6: AI Draft Quality ━━━${NC}"
echo -e "  Testing: Generated drafts are grounded in real org docs\n"

DRAFTS=$(curl -s "$API_URL/api/v1/autopilot/drafts" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null)

if [ -z "$DRAFTS" ] || echo "$DRAFTS" | grep -q "error\|Error"; then
  fail "6.1 Draft API availability" "API error"
else
  DRAFT_COUNT=$(echo "$DRAFTS" | python3 -c "import json,sys; print(len(json.load(sys.stdin)))" 2>/dev/null || echo "0")

  if [ "$DRAFT_COUNT" -ge 1 ]; then
    pass "6.1 Drafts available ($DRAFT_COUNT drafts)"

    # Analyze the first (most recent) draft
    echo "$DRAFTS" | python3 -c "
import json, sys

drafts = json.load(sys.stdin)
draft = drafts[0]
content = draft.get('content', '')
title = draft.get('title', 'Unknown')

# Count grounding signals
source_citations = content.lower().count('source:')
gap_markers = content.count('[GAP]')
has_related_docs = 'Related Documentation' in content or 'Related Documents' in content
has_open_questions = 'Open Questions' in content
word_count = len(content.split())

print(f'TITLE:{title}')
print(f'WORDS:{word_count}')
print(f'CITATIONS:{source_citations}')
print(f'GAPS:{gap_markers}')
print(f'RELATED:{has_related_docs}')
print(f'QUESTIONS:{has_open_questions}')
" 2>/dev/null > /tmp/docbrain_draft_analysis.txt

    DRAFT_TITLE=$(grep "^TITLE:" /tmp/docbrain_draft_analysis.txt | cut -d: -f2-)
    DRAFT_WORDS=$(grep "^WORDS:" /tmp/docbrain_draft_analysis.txt | cut -d: -f2)
    DRAFT_CITATIONS=$(grep "^CITATIONS:" /tmp/docbrain_draft_analysis.txt | cut -d: -f2)
    DRAFT_GAPS=$(grep "^GAPS:" /tmp/docbrain_draft_analysis.txt | cut -d: -f2)
    DRAFT_RELATED=$(grep "^RELATED:" /tmp/docbrain_draft_analysis.txt | cut -d: -f2)
    DRAFT_QUESTIONS=$(grep "^QUESTIONS:" /tmp/docbrain_draft_analysis.txt | cut -d: -f2)

    echo -e "    Analyzing draft: ${CYAN}$DRAFT_TITLE${NC}"

    # 6.2: Draft should have source citations
    if [ "$DRAFT_CITATIONS" -ge 3 ]; then
      pass "6.2 Source citations ($DRAFT_CITATIONS citations)"
    elif [ "$DRAFT_CITATIONS" -ge 1 ]; then
      warn "6.2 Source citations" "Only $DRAFT_CITATIONS citations (expected 3+)"
    else
      fail "6.2 Source citations" "No source citations — draft is ungrounded"
    fi

    # 6.3: Draft should have GAP markers where info is missing
    if [ "$DRAFT_GAPS" -ge 1 ]; then
      pass "6.3 GAP markers present ($DRAFT_GAPS gaps marked)"
    else
      warn "6.3 GAP markers" "No [GAP] markers — suspicious completeness"
    fi

    # 6.4: Draft should have Related Documentation section
    if [ "$DRAFT_RELATED" = "True" ]; then
      pass "6.4 Related Documentation section present"
    else
      fail "6.4 Related Documentation" "Missing Related Documentation section"
    fi

    # 6.5: Draft should have Open Questions section
    if [ "$DRAFT_QUESTIONS" = "True" ]; then
      pass "6.5 Open Questions section present"
    else
      fail "6.5 Open Questions" "Missing Open Questions section"
    fi

    # 6.6: Draft should be substantial (500+ words)
    if [ "$DRAFT_WORDS" -ge 500 ]; then
      pass "6.6 Draft substance ($DRAFT_WORDS words)"
    elif [ "$DRAFT_WORDS" -ge 200 ]; then
      warn "6.6 Draft substance" "Only $DRAFT_WORDS words (expected 500+)"
    else
      fail "6.6 Draft substance" "Only $DRAFT_WORDS words — too thin"
    fi

    # 6.7: Draft should NOT contain boilerplate markers
    echo "$DRAFTS" | python3 -c "
import json, sys
drafts = json.load(sys.stdin)
content = drafts[0].get('content', '').lower()
boilerplate = [
    'your-service', 'example.com', 'my-app', 'your-app',
    'replace with', 'todo:', 'xxx', 'placeholder',
    'lorem ipsum', 'acme corp'
]
found = [b for b in boilerplate if b in content]
print(','.join(found) if found else 'NONE')
" 2>/dev/null > /tmp/docbrain_boilerplate.txt

    BOILERPLATE=$(cat /tmp/docbrain_boilerplate.txt)
    if [ "$BOILERPLATE" = "NONE" ]; then
      pass "6.7 No boilerplate markers"
    else
      fail "6.7 Boilerplate detected" "Found: $BOILERPLATE"
    fi
  else
    warn "6.1 Draft availability" "No drafts generated yet"
    echo -e "    ${YELLOW}Skipping draft quality tests (no drafts to analyze)${NC}"
  fi
fi

echo ""

# ============================================================================
# PAIN POINT 7: Source Attribution — "Where did that answer come from?"
# ============================================================================
echo -e "${BOLD}━━━ Pain Point 7: Source Attribution & Traceability ━━━${NC}"
echo -e "  Testing: Answers include traceable source URLs\n"

# Test with a question likely to have good sources
RESPONSE=$(ask "How do we handle incident escalation?")
if echo "$RESPONSE" | grep -q "ERROR:"; then
  fail "7.1 Source attribution" "API error"
else
  echo "$RESPONSE" | python3 -c "
import json, sys
d = json.load(sys.stdin)
sources = d.get('sources', [])
with_url = sum(1 for s in sources if s.get('source_url'))
with_title = sum(1 for s in sources if s.get('title'))
with_score = sum(1 for s in sources if s.get('score', 0) > 0)
print(f'TOTAL:{len(sources)}')
print(f'WITH_URL:{with_url}')
print(f'WITH_TITLE:{with_title}')
print(f'WITH_SCORE:{with_score}')
" 2>/dev/null > /tmp/docbrain_sources.txt

  SRC_TOTAL=$(grep "^TOTAL:" /tmp/docbrain_sources.txt | cut -d: -f2)
  SRC_URL=$(grep "^WITH_URL:" /tmp/docbrain_sources.txt | cut -d: -f2)
  SRC_TITLE=$(grep "^WITH_TITLE:" /tmp/docbrain_sources.txt | cut -d: -f2)
  SRC_SCORE=$(grep "^WITH_SCORE:" /tmp/docbrain_sources.txt | cut -d: -f2)

  # 7.1: Should have sources
  if [ "$SRC_TOTAL" -ge 3 ]; then
    pass "7.1 Sources returned ($SRC_TOTAL sources)"
  elif [ "$SRC_TOTAL" -ge 1 ]; then
    warn "7.1 Sources returned" "Only $SRC_TOTAL sources (expected 3+)"
  else
    fail "7.1 Sources returned" "No sources in response"
  fi

  # 7.2: Sources should have URLs
  if [ "$SRC_URL" = "$SRC_TOTAL" ] && [ "$SRC_TOTAL" != "0" ]; then
    pass "7.2 All sources have URLs ($SRC_URL/$SRC_TOTAL)"
  elif [ "$SRC_URL" -gt 0 ]; then
    warn "7.2 Source URLs" "$SRC_URL/$SRC_TOTAL sources have URLs"
  else
    fail "7.2 Source URLs" "No sources have URLs"
  fi

  # 7.3: Sources should have titles
  if [ "$SRC_TITLE" = "$SRC_TOTAL" ] && [ "$SRC_TOTAL" != "0" ]; then
    pass "7.3 All sources have titles ($SRC_TITLE/$SRC_TOTAL)"
  else
    warn "7.3 Source titles" "$SRC_TITLE/$SRC_TOTAL sources have titles"
  fi

  # 7.4: Sources should have relevance scores
  if [ "$SRC_SCORE" = "$SRC_TOTAL" ] && [ "$SRC_TOTAL" != "0" ]; then
    pass "7.4 All sources have relevance scores ($SRC_SCORE/$SRC_TOTAL)"
  else
    warn "7.4 Source scores" "$SRC_SCORE/$SRC_TOTAL sources have scores"
  fi
fi

# 7.5: Feedback mechanism works
EPISODE_ID=$(echo "$RESPONSE1" | python3 -c "import json,sys; print(json.load(sys.stdin).get('episode_id',''))" 2>/dev/null)
if [ -n "$EPISODE_ID" ] && [ "$EPISODE_ID" != "null" ] && [ "$EPISODE_ID" != "" ]; then
  FB_RESULT=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/v1/feedback" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer $API_KEY" \
    -d "{\"episode_id\": \"$EPISODE_ID\", \"feedback\": 1}" 2>/dev/null)

  if [ "$FB_RESULT" = "200" ]; then
    pass "7.5 Feedback submission works"
  else
    fail "7.5 Feedback submission" "HTTP $FB_RESULT"
  fi
else
  warn "7.5 Feedback submission" "No episode_id to test with"
fi

echo ""

# ============================================================================
# PAIN POINT BONUS: Analytics & Observability
# ============================================================================
echo -e "${BOLD}━━━ Bonus: Analytics & Health ━━━${NC}"
echo -e "  Testing: Platform observability features\n"

# Analytics
ANALYTICS=$(curl -s "$API_URL/api/v1/analytics?days=30" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null)

if echo "$ANALYTICS" | python3 -c "import json,sys; d=json.load(sys.stdin); assert d['total_queries'] >= 0" 2>/dev/null; then
  TOTAL_Q=$(echo "$ANALYTICS" | python3 -c "import json,sys; print(json.load(sys.stdin)['total_queries'])" 2>/dev/null)
  pass "B.1 Analytics endpoint works (${TOTAL_Q} queries tracked)"
else
  fail "B.1 Analytics endpoint" "API error"
fi

# Health dashboard
HEALTH_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/v1/health/report" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null)
if [ "$HEALTH_CODE" = "200" ]; then
  pass "B.2 Health dashboard endpoint works"
else
  fail "B.2 Health dashboard" "HTTP $HEALTH_CODE"
fi

# Config features
FEATURE_COUNT=$(echo "$CONFIG" | python3 -c "import json,sys; d=json.load(sys.stdin); print(sum(1 for v in d.get('features',{}).values() if v))" 2>/dev/null || echo "0")
if [ "$FEATURE_COUNT" -ge 8 ]; then
  pass "B.3 Feature flags ($FEATURE_COUNT features enabled)"
else
  warn "B.3 Feature flags" "Only $FEATURE_COUNT features enabled"
fi

# Weekly digest
DIGEST=$(curl -s "$API_URL/api/v1/autopilot/digest" \
  -H "Authorization: Bearer $API_KEY" 2>/dev/null)

if echo "$DIGEST" | python3 -c "import json,sys; d=json.load(sys.stdin); assert 'period_start' in d" 2>/dev/null; then
  pass "B.4 Weekly digest endpoint works"
else
  DIGEST_CODE=$(curl -s -o /dev/null -w "%{http_code}" "$API_URL/api/v1/autopilot/digest" \
    -H "Authorization: Bearer $API_KEY" 2>/dev/null)
  if [ "$DIGEST_CODE" = "200" ]; then
    pass "B.4 Weekly digest endpoint works"
  else
    fail "B.4 Weekly digest" "HTTP $DIGEST_CODE"
  fi
fi

echo ""

# ============================================================================
# RESULTS SUMMARY
# ============================================================================
echo -e "${BOLD}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║  RESULTS SUMMARY                                             ║${NC}"
echo -e "${BOLD}╚══════════════════════════════════════════════════════════════╝${NC}\n"

TOTAL=$((PASS + FAIL + WARN))

echo -e "  ${GREEN}PASS: $PASS${NC}  |  ${RED}FAIL: $FAIL${NC}  |  ${YELLOW}WARN: $WARN${NC}  |  Total: $TOTAL\n"

if [ $FAIL -eq 0 ] && [ $WARN -eq 0 ]; then
  echo -e "  ${GREEN}${BOLD}ALL TESTS PASSED${NC} — Every pain point addressed.\n"
elif [ $FAIL -eq 0 ]; then
  echo -e "  ${GREEN}${BOLD}NO FAILURES${NC} — $WARN warnings to investigate.\n"
else
  echo -e "  ${RED}${BOLD}$FAIL FAILURES${NC} — Issues need attention.\n"
fi

echo -e "${BOLD}Detailed Results:${NC}"
for result in "${RESULTS[@]}"; do
  echo -e "  $result"
done

echo ""

# Score calculation
SCORE=$(python3 -c "
pass_val = $PASS
warn_val = $WARN
fail_val = $FAIL
total = pass_val + warn_val + fail_val
if total == 0:
    print('N/A')
else:
    score = (pass_val * 100 + warn_val * 50) / total
    print(f'{score:.0f}/100')
")

echo -e "${BOLD}Overall Score: $SCORE${NC}"
echo ""

# Write machine-readable results
cat > /tmp/docbrain_test_results.json << JSONEOF
{
  "date": "$(date -u '+%Y-%m-%dT%H:%M:%SZ')",
  "server": "$API_URL",
  "pass": $PASS,
  "fail": $FAIL,
  "warn": $WARN,
  "total": $TOTAL,
  "score": "$SCORE"
}
JSONEOF

echo "Machine-readable results: /tmp/docbrain_test_results.json"

# Exit code: 0 if no failures, 1 if any failures
[ $FAIL -eq 0 ]
