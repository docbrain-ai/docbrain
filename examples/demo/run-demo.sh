#!/usr/bin/env bash
set -euo pipefail
# ═══════════════════════════════════════════════════════════════════════════
# DocBrain Feature Showcase — Complete Demo Runner
#
# Starts all services, ingests documents, seeds demo data, and opens the UI.
# Designed to be a one-command setup for the video walkthrough.
#
# Usage:
#   bash examples/demo/run-demo.sh
# ═══════════════════════════════════════════════════════════════════════════

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$SCRIPT_DIR/../.."
cd "$PROJECT_DIR"

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

echo ""
echo -e "${BLUE}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║         ${BOLD}DocBrain Feature Showcase — Demo Setup${NC}${BLUE}              ║${NC}"
echo -e "${BLUE}║  Shift-Left Documentation • Knowledge Capture • AI Quality  ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════════════════════╝${NC}"
echo ""

# ── Step 1: Check prerequisites ──────────────────────────────────────────────
echo -e "${BOLD}Step 1: Checking prerequisites${NC}"

if ! command -v docker &> /dev/null; then
    echo -e "${YELLOW}Error:${NC} Docker is not installed."
    exit 1
fi

if ! docker compose version &> /dev/null; then
    echo -e "${YELLOW}Error:${NC} Docker Compose V2 is not available."
    exit 1
fi

if [ ! -f .env ]; then
    echo -e "${YELLOW}No .env file found. Creating from template...${NC}"
    cp .env.example .env
    echo -e "${YELLOW}Please edit .env to set your LLM provider and API key, then re-run.${NC}"
    exit 1
fi

echo -e "  ${GREEN}✓${NC} Docker and Compose available"
echo -e "  ${GREEN}✓${NC} .env file exists"

# ── Step 2: Start services ───────────────────────────────────────────────────
echo -e "\n${BOLD}Step 2: Starting services${NC}"
docker compose up -d --build

echo -n "  Waiting for PostgreSQL"
for i in $(seq 1 30); do
    if docker compose exec -T postgres pg_isready -U docbrain > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

echo -n "  Waiting for OpenSearch"
for i in $(seq 1 45); do
    if curl -sf http://localhost:9200/_cluster/health > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

echo -n "  Waiting for DocBrain server"
for i in $(seq 1 45); do
    if curl -sf http://localhost:3000/api/v1/health > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

echo -n "  Waiting for wiki connector"
for i in $(seq 1 20); do
    if curl -sf http://localhost:4000/health > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

# ── Step 3: Ingest sample docs ───────────────────────────────────────────────
echo -e "\n${BOLD}Step 3: Ingesting sample documents${NC}"
INDEX_COUNT=$(curl -sf "http://localhost:9200/docbrain-chunks/_count" 2>/dev/null | grep -o '"count":[0-9]*' | cut -d: -f2 || echo "0")

if [[ "${INDEX_COUNT:-0}" -eq 0 ]]; then
    echo "  Ingesting 13 sample documents..."
    docker compose exec -T server docbrain-ingest 2>&1 | tail -5
    NEW_COUNT=$(curl -sf "http://localhost:9200/docbrain-chunks/_count" 2>/dev/null | grep -o '"count":[0-9]*' | cut -d: -f2 || echo "0")
    echo -e "  ${GREEN}✓${NC} Ingested ${NEW_COUNT} chunks"
else
    echo -e "  ${GREEN}✓${NC} Documents already ingested (${INDEX_COUNT} chunks)"
fi

# ── Step 4: Get admin key ────────────────────────────────────────────────────
echo -e "\n${BOLD}Step 4: Retrieving admin API key${NC}"
BOOTSTRAP_KEY=""
if docker compose exec -T server test -f /app/admin-bootstrap-key.txt 2>/dev/null; then
    BOOTSTRAP_KEY=$(docker compose exec -T server cat /app/admin-bootstrap-key.txt 2>/dev/null | grep "^Key:" | cut -d' ' -f2 || true)
fi

if [ -n "$BOOTSTRAP_KEY" ]; then
    echo -e "  ${GREEN}✓${NC} Admin key: ${BOOTSTRAP_KEY}"
    export DOCBRAIN_API_KEY="$BOOTSTRAP_KEY"
else
    echo -e "  ${YELLOW}!${NC} Could not retrieve bootstrap key. Set DOCBRAIN_API_KEY manually."
    if [ -z "${DOCBRAIN_API_KEY:-}" ]; then
        echo -e "  ${YELLOW}Skipping seed step.${NC}"
    fi
fi

# ── Step 5: Seed demo data ───────────────────────────────────────────────────
if [ -n "${DOCBRAIN_API_KEY:-}" ]; then
    echo -e "\n${BOLD}Step 5: Seeding demo data${NC}"
    bash examples/demo/seed.sh
else
    echo -e "\n${YELLOW}Step 5: Skipped (no API key). Run seed.sh manually after setting DOCBRAIN_API_KEY.${NC}"
fi

# ── Done ─────────────────────────────────────────────────────────────────────
echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║            Demo environment is ready!                        ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  ${BOLD}Web UI:${NC}       http://localhost:3001"
echo -e "  ${BOLD}API Server:${NC}   http://localhost:3000"
echo -e "  ${BOLD}Swagger UI:${NC}   http://localhost:3000/api/docs"
echo -e "  ${BOLD}Connector:${NC}    http://localhost:4000/health"
echo ""
echo -e "  ${BOLD}Login:${NC}        admin@acme.com / DemoPassword123!"
echo ""
echo -e "  ${BOLD}Video Script:${NC} examples/demo/VIDEO_WALKTHROUGH.md"
echo ""
echo -e "  ${BOLD}Useful commands:${NC}"
echo "    docker compose logs -f server     # Server logs"
echo "    docker compose logs -f web        # Web UI logs"
echo "    docker compose down               # Stop everything"
echo ""
