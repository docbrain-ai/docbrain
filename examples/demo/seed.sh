#!/usr/bin/env bash
set -euo pipefail
# ═══════════════════════════════════════════════════════════════════════════
# DocBrain Feature Showcase — Demo Seed Script
#
# Populates a running DocBrain instance with realistic data so every
# dashboard, feature, and workflow has something to show during a demo.
#
# Prerequisites:
#   - DocBrain running (docker compose up -d)
#   - Admin API key available
#   - Documents already ingested (docbrain-ingest)
#
# Usage:
#   export DOCBRAIN_API_KEY="db_sk_..."
#   bash examples/demo/seed.sh
# ═══════════════════════════════════════════════════════════════════════════

API="${DOCBRAIN_API_URL:-http://localhost:3000}"
KEY="${DOCBRAIN_API_KEY:?Set DOCBRAIN_API_KEY to your admin API key}"
AUTH="Authorization: Bearer $KEY"
CT="Content-Type: application/json"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

ok()   { echo -e "  ${GREEN}✓${NC} $1"; }
info() { echo -e "  ${BLUE}→${NC} $1"; }
warn() { echo -e "  ${YELLOW}!${NC} $1"; }

api() {
  local method="$1" path="$2"
  shift 2
  curl -sf -X "$method" "$API$path" -H "$AUTH" -H "$CT" "$@" 2>/dev/null || true
}

echo ""
echo -e "${BOLD}╔══════════════════════════════════════════════════════╗${NC}"
echo -e "${BOLD}║       DocBrain Feature Showcase — Data Seed         ║${NC}"
echo -e "${BOLD}╚══════════════════════════════════════════════════════╝${NC}"
echo ""

# ── 1. Create admin user ─────────────────────────────────────────────────────
echo -e "${BOLD}1. Admin User${NC}"
ADMIN_RESP=$(api POST /api/v1/admin/users -d '{
  "email": "admin@acme.com",
  "password": "DemoPassword123!",
  "display_name": "Admin",
  "role": "admin"
}')
if echo "$ADMIN_RESP" | grep -q "user_id\|already"; then
  ok "Admin user created (admin@acme.com)"
else
  warn "Admin user may already exist — continuing"
fi

# ── 2. Create API keys for different roles ────────────────────────────────────
echo -e "\n${BOLD}2. API Keys (RBAC demo)${NC}"
api POST /api/v1/admin/keys -d '{
  "name": "Platform Team (Editor)",
  "role": "editor",
  "allowed_spaces": ["PLATFORM", "INFRA"]
}' > /dev/null && ok "Created Platform Team editor key"

api POST /api/v1/admin/keys -d '{
  "name": "Payments Read-Only",
  "role": "viewer",
  "allowed_spaces": ["PAYMENTS"]
}' > /dev/null && ok "Created Payments viewer key"

api POST /api/v1/admin/keys -d '{
  "name": "Analytics Analyst",
  "role": "analyst",
  "allowed_spaces": []
}' > /dev/null && ok "Created Analyst key (all spaces)"

# ── 3. Ask questions to build episodic memory ─────────────────────────────────
echo -e "\n${BOLD}3. Seeding Q&A history (episodic memory)${NC}"
QUESTIONS=(
  "How do I deploy to production?"
  "What is the rollback procedure?"
  "How do I connect to the production database?"
  "What are the Redis OOM troubleshooting steps?"
  "How do I create a new Kubernetes ingress route?"
  "What is the incident response process for SEV-1?"
  "How do I rotate secrets in Vault?"
  "What is our API authentication method?"
  "How do canary deployments work?"
  "What is the payment refund process?"
  "Who is on-call for the payments service?"
  "How do I add a new feature flag?"
  "What are the CI/CD pipeline stages?"
  "How do I check database connection pool usage?"
  "What monitoring dashboards are available?"
)

for q in "${QUESTIONS[@]}"; do
  RESP=$(api POST /api/v1/ask -d "{\"question\": \"$q\"}")
  EPISODE_ID=$(echo "$RESP" | grep -o '"episode_id":"[^"]*"' | head -1 | cut -d'"' -f4)
  if [ -n "$EPISODE_ID" ]; then
    ok "Asked: $q"
  else
    warn "Question may have failed: $q"
  fi
done

# ── 4. Submit feedback (positive & negative for autopilot) ────────────────────
echo -e "\n${BOLD}4. Submitting feedback (seeds autopilot gap detection)${NC}"
info "Submitting positive feedback for good answers..."
info "Submitting negative feedback to trigger gap detection..."

# Ask questions that should create negative signals (topics not well covered)
GAP_QUESTIONS=(
  "How do I set up a new microservice from scratch?"
  "What is our data backup and disaster recovery plan?"
  "How do I configure cross-region replication?"
  "What are our GDPR compliance procedures?"
  "How do I debug a memory leak in production?"
)

for q in "${GAP_QUESTIONS[@]}"; do
  RESP=$(api POST /api/v1/ask -d "{\"question\": \"$q\"}")
  EPISODE_ID=$(echo "$RESP" | grep -o '"episode_id":"[^"]*"' | head -1 | cut -d'"' -f4)
  if [ -n "$EPISODE_ID" ]; then
    api POST /api/v1/feedback -d "{\"episode_id\": \"$EPISODE_ID\", \"feedback\": -1}" > /dev/null
    ok "Negative feedback: $q"
  fi
done

# ── 5. Create knowledge fragments (shift-left capture) ────────────────────────
echo -e "\n${BOLD}5. Creating knowledge fragments${NC}"

api POST /api/v1/fragments -d '{
  "fragment_type": "decision",
  "summary": "Switched event delivery from Redis pub/sub to PostgreSQL LISTEN/NOTIFY",
  "content": "Redis cluster mode does not support pub/sub across shards. After evaluating alternatives (Kafka, NATS, PG LISTEN/NOTIFY), we chose PG LISTEN/NOTIFY for simplicity since all services already connect to PostgreSQL. Trade-off: limited to ~10K notifications/sec, which is sufficient for our current event volume. If we exceed this, we will migrate to Kafka.",
  "source_type": "pr_merge",
  "source_ref": "https://github.com/acme/platform/pull/1234",
  "source_id": "github:acme/platform#1234",
  "confidence": 0.92,
  "space": "PLATFORM",
  "code_location": "src/events/publisher.rs:42"
}' > /dev/null && ok "Fragment: Redis → PG LISTEN/NOTIFY decision"

api POST /api/v1/fragments -d '{
  "fragment_type": "caveat",
  "summary": "Payment retries must use idempotency keys to avoid double-charging",
  "content": "When retrying failed payment API calls, always include the same idempotency key. If you generate a new key on each retry, Stripe will treat each attempt as a separate payment and the customer will be charged multiple times. The idempotency key should be deterministic — use the order ID, not a random UUID.",
  "source_type": "pr_merge",
  "source_ref": "https://github.com/acme/platform/pull/1456",
  "source_id": "github:acme/platform#1456",
  "confidence": 0.88,
  "space": "PAYMENTS"
}' > /dev/null && ok "Fragment: Idempotency key caveat"

api POST /api/v1/fragments -d '{
  "fragment_type": "procedure",
  "summary": "Emergency database failover procedure for RDS Multi-AZ",
  "content": "To trigger a manual failover: 1) Log into AWS Console → RDS → Instances → acme-prod. 2) Click Actions → Reboot → Check \"Reboot with failover\". 3) Failover takes 60-120 seconds. 4) Verify via: aws rds describe-events --source-identifier acme-prod --duration 60. 5) All connections will drop — application connection pools handle reconnection automatically. 6) DNS endpoint stays the same, but the underlying IP changes.",
  "source_type": "incident",
  "source_ref": "INC-2026-0305",
  "confidence": 0.95,
  "space": "INFRA"
}' > /dev/null && ok "Fragment: RDS failover procedure"

api POST /api/v1/fragments -d '{
  "fragment_type": "fact",
  "summary": "Kubernetes pod DNS resolution uses ndots=5 by default, causing slow lookups",
  "content": "By default, Kubernetes sets ndots=5 in /etc/resolv.conf inside pods. This means any hostname with fewer than 5 dots will be searched through the search domains first (e.g., payments-api becomes payments-api.default.svc.cluster.local, payments-api.svc.cluster.local, etc.) before trying the absolute name. For external hostnames, this causes 4-5 unnecessary DNS queries. Set ndots=2 in your pod spec dnsConfig to fix this.",
  "source_type": "conversation_distill",
  "source_ref": "slack://C01234/thread-1234",
  "confidence": 0.82,
  "space": "PLATFORM"
}' > /dev/null && ok "Fragment: K8s DNS ndots fact"

api POST /api/v1/fragments -d '{
  "fragment_type": "context",
  "summary": "Feature flags older than 30 days after full rollout must be cleaned up",
  "content": "The Feature Flag Committee reviews stale flags monthly. Any flag that has been at 100% rollout for more than 30 days is considered stale. The flag owner receives a Slack notification and has 1 sprint to remove the flag from code. Unresolved stale flags are escalated to the engineering manager. This policy was introduced after the incident where a stale flag caused a regression when its dependency was removed.",
  "source_type": "manual",
  "confidence": 0.78,
  "space": "PLATFORM"
}' > /dev/null && ok "Fragment: Feature flag cleanup policy"

api POST /api/v1/fragments -d '{
  "fragment_type": "decision",
  "summary": "Chose Trivy over Snyk for container security scanning",
  "content": "We evaluated Trivy and Snyk for container image scanning in our CI pipeline. Trivy was chosen because: 1) It is open source with no per-scan licensing costs. 2) It scans both OS packages and application dependencies in a single pass. 3) It integrates natively with GitHub Actions. 4) Offline DB updates are supported for air-gapped environments. Trade-off: Snyk has better SBOM management and a developer-friendly web UI, but the cost ($15/dev/month) was not justified for our team size.",
  "source_type": "pr_merge",
  "source_ref": "https://github.com/acme/platform/pull/1789",
  "source_id": "github:acme/platform#1789",
  "confidence": 0.90,
  "space": "PLATFORM"
}' > /dev/null && ok "Fragment: Trivy vs Snyk decision"

# ── 6. CI/CD capture (simulating merged PRs) ──────────────────────────────────
echo -e "\n${BOLD}6. CI/CD pipeline captures${NC}"

api POST /api/v1/ci/analyze -d '{
  "pr_number": 2847,
  "repo": "acme/platform",
  "pr_title": "Add zero-downtime API key rotation",
  "pr_body": "Implements zero-downtime API key rotation by supporting multiple active keys simultaneously with a 24h grace period. Includes automated smoke testing before activation.",
  "diff_stat": "+340 -45",
  "changed_files": "src/auth/key_rotation.rs,src/auth/middleware.rs,tests/auth_rotation_test.rs",
  "labels": "security,breaking-change",
  "author": "alex.kim@acme.com"
}' > /dev/null && ok "CI capture: API key rotation PR #2847"

api POST /api/v1/ci/analyze -d '{
  "pr_number": 2851,
  "repo": "acme/platform",
  "pr_title": "Migrate payments to EventBridge",
  "pr_body": "Replaces synchronous HTTP calls to downstream services with EventBridge domain events. Payment state changes are now published as events consumed by notifications, analytics, and reconciliation services.",
  "diff_stat": "+890 -320",
  "changed_files": "src/payments/events.rs,src/payments/handler.rs,infra/eventbridge.tf",
  "labels": "architecture,payments",
  "author": "marcus@acme.com"
}' > /dev/null && ok "CI capture: EventBridge migration PR #2851"

api POST /api/v1/ci/deploy-capture -d '{
  "service": "payments-api",
  "version": "2.8.0",
  "environment": "production",
  "changelog": "feat: event-driven payment notifications\nfix: idempotency key collision on retry\nchore: upgrade Stripe SDK to v14",
  "config_diff": "EVENT_BRIDGE_ENABLED: false -> true\nSTRIPE_SDK_VERSION: 13.2 -> 14.0"
}' > /dev/null && ok "Deploy capture: payments-api v2.8.0 → production"

# ── 7. Import style rules ────────────────────────────────────────────────────
echo -e "\n${BOLD}7. Importing style rules${NC}"
RULES_YAML=$(cat examples/demo/style-rules.yaml)
api POST /api/v1/style-rules/import -d "{\"yaml\": $(echo "$RULES_YAML" | python3 -c 'import sys,json; print(json.dumps(sys.stdin.read()))')}" > /dev/null && ok "Imported style rules from YAML"

# ── 8. Register the wiki connector ────────────────────────────────────────────
echo -e "\n${BOLD}8. External connector${NC}"
api POST /api/v1/connectors -d '{
  "name": "acme-wiki",
  "display_name": "Acme Internal Wiki",
  "base_url": "http://wiki-connector:4000",
  "source_type": "wiki",
  "schedule_cron": "0 */6 * * *",
  "space": "ENGINEERING"
}' > /dev/null && ok "Registered wiki connector"

# ── 9. Create a webhook subscription ──────────────────────────────────────────
echo -e "\n${BOLD}9. Webhook subscription${NC}"
api POST /api/v1/webhooks -d '{
  "name": "Demo Event Logger",
  "url": "https://webhook.site/demo-docbrain",
  "secret": "demo-secret-at-least-16-chars",
  "events": ["gap.detected", "draft.generated", "fragment.captured", "sla.breached"],
  "headers": {"X-Source": "docbrain-demo"}
}' > /dev/null && ok "Created webhook subscription"

# ── 10. Trigger autopilot analysis ────────────────────────────────────────────
echo -e "\n${BOLD}10. Triggering Autopilot gap analysis${NC}"
api POST /api/v1/autopilot/analyze > /dev/null && ok "Autopilot analysis triggered"

# ── Summary ───────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║        Demo data seeded successfully!                ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  ${BOLD}What was created:${NC}"
echo "    • Admin user + 3 role-specific API keys"
echo "    • 15 questions in episodic memory"
echo "    • 5 negative-feedback signals (for gap detection)"
echo "    • 6 knowledge fragments (decisions, caveats, procedures)"
echo "    • 2 CI/CD PR captures + 1 deploy capture"
echo "    • 12 style rules imported"
echo "    • 1 external connector registered"
echo "    • 1 webhook subscription"
echo "    • Autopilot gap analysis triggered"
echo ""
echo -e "  ${BOLD}Open the Web UI:${NC} http://localhost:3001"
echo -e "  ${BOLD}Login:${NC} admin@acme.com / DemoPassword123!"
echo ""
