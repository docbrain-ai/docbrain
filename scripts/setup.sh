#!/usr/bin/env bash
set -euo pipefail

# ═══════════════════════════════════════════════════════════════════════════
# DocBrain — Interactive Setup Wizard
# ═══════════════════════════════════════════════════════════════════════════

GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
BOLD='\033[1m'
NC='\033[0m'

echo ""
echo -e "${BLUE}╔══════════════════════════════════════════════╗${NC}"
echo -e "${BLUE}║           ${BOLD}DocBrain Setup Wizard${NC}${BLUE}              ║${NC}"
echo -e "${BLUE}║    AI-Powered Documentation Intelligence     ║${NC}"
echo -e "${BLUE}╚══════════════════════════════════════════════╝${NC}"
echo ""

# ── Pre-flight checks ────────────────────────────────────────────────────

if ! command -v docker &> /dev/null; then
    echo -e "${YELLOW}Error:${NC} Docker is not installed."
    echo "  Install: https://docs.docker.com/get-docker/"
    exit 1
fi

if ! docker compose version &> /dev/null; then
    echo -e "${YELLOW}Error:${NC} Docker Compose V2 is not available."
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR/.."

SKIP_SETUP=false

if [ -f .env ]; then
    echo -e "${YELLOW}An .env file already exists.${NC}"
    read -rp "Overwrite? (y/N): " overwrite
    if [[ "$overwrite" != "y" && "$overwrite" != "Y" ]]; then
        echo ""
        echo "Starting services with existing configuration..."
        docker compose up -d
        # Fall through to the startup wait and instructions below
        SKIP_SETUP=true
    fi
fi

# Ensure config/local.yaml exists (gitignored — safe for secrets and overrides)
if [ ! -f config/local.yaml ]; then
    mkdir -p config
    cat > config/local.yaml << 'LOCALYAML'
# config/local.yaml — never committed (gitignored)
# Use this file for ingest source credentials and personal overrides.
# Infrastructure secrets (DATABASE_URL, ANTHROPIC_API_KEY, etc.) stay in .env.
#
# Example:
#
# ingest:
#   ingest_sources: confluence,github_pr
#
# confluence:
#   base_url: https://acme.atlassian.net/wiki
#   user_email: you@acme.com
#   api_token: ATATT3x...
#   space_keys: DOCS,ENG
#
# github_pr:
#   token: ghp_...
#   repo: acme/platform
#   lookback_days: 180
LOCALYAML
fi

if [ "$SKIP_SETUP" = false ]; then

cp .env.example .env

# ── LLM Provider ─────────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}Step 1/3: LLM Provider${NC}"
echo ""
echo "  1) Anthropic (Claude)        — best quality, requires API key"
echo "  2) OpenAI (GPT-4o)           — requires API key"
echo "  3) Ollama (100% local)       — no API key, runs on your hardware"
echo "  4) AWS Bedrock               — requires AWS credentials"
echo ""
read -rp "Select provider [1]: " llm_choice
llm_choice=${llm_choice:-1}

case $llm_choice in
    1)
        sed -i.bak 's/^LLM_PROVIDER=.*/LLM_PROVIDER=anthropic/' .env
        sed -i.bak 's/^LLM_MODEL_ID=.*/LLM_MODEL_ID=claude-sonnet-4-5-20250929/' .env
        echo ""
        read -rp "  Anthropic API key: " api_key
        sed -i.bak "s/^# ANTHROPIC_API_KEY=.*/ANTHROPIC_API_KEY=${api_key}/" .env
        ;;
    2)
        sed -i.bak 's/^LLM_PROVIDER=.*/LLM_PROVIDER=openai/' .env
        sed -i.bak 's/^LLM_MODEL_ID=.*/LLM_MODEL_ID=gpt-4o/' .env
        echo ""
        read -rp "  OpenAI API key: " api_key
        # Uncomment and set
        sed -i.bak "s/^# OPENAI_API_KEY=.*/OPENAI_API_KEY=${api_key}/" .env
        ;;
    3)
        sed -i.bak 's/^LLM_PROVIDER=.*/LLM_PROVIDER=ollama/' .env
        sed -i.bak 's/^LLM_MODEL_ID=.*/LLM_MODEL_ID=command-r:35b/' .env
        sed -i.bak 's/^# OLLAMA_BASE_URL=.*/OLLAMA_BASE_URL=http:\/\/host.docker.internal:11434/' .env
        sed -i.bak 's/^EMBED_PROVIDER=.*/EMBED_PROVIDER=ollama/' .env
        sed -i.bak 's/^EMBED_MODEL_ID=.*/EMBED_MODEL_ID=nomic-embed-text/' .env
        echo ""
        echo -e "  ${YELLOW}Ensure Ollama is running with the required models:${NC}"
        echo "    ollama pull command-r:35b"
        echo "    ollama pull nomic-embed-text"
        echo ""
        echo -e "  ${YELLOW}Note:${NC} command-r:35b requires ~24 GB RAM. If your machine has less,"
        echo "  use a cloud provider (Anthropic/OpenAI) or set LLM_MODEL_ID manually."
        ;;
    4)
        sed -i.bak 's/^LLM_PROVIDER=.*/LLM_PROVIDER=bedrock/' .env
        echo ""
        read -rp "  AWS Region [us-east-1]: " aws_region
        aws_region=${aws_region:-us-east-1}
        sed -i.bak "s/^AWS_REGION=.*/AWS_REGION=${aws_region}/" .env
        read -rp "  AWS Access Key ID: " aws_key
        sed -i.bak "s/^# AWS_ACCESS_KEY_ID=.*/AWS_ACCESS_KEY_ID=${aws_key}/" .env
        read -rp "  AWS Secret Access Key: " aws_secret
        sed -i.bak "s/^# AWS_SECRET_ACCESS_KEY=.*/AWS_SECRET_ACCESS_KEY=${aws_secret}/" .env
        sed -i.bak 's/^EMBED_PROVIDER=.*/EMBED_PROVIDER=bedrock/' .env
        sed -i.bak 's/^EMBED_MODEL_ID=.*/EMBED_MODEL_ID=cohere.embed-v4:0/' .env
        ;;
    *)
        echo "Invalid choice. Using Anthropic as default."
        ;;
esac

# ── Embedding Provider (skip if already set by Ollama/Bedrock) ───────────

if [[ "$llm_choice" != "3" && "$llm_choice" != "4" ]]; then
    echo ""
    echo -e "${BOLD}Step 2/3: Embedding Provider${NC}"
    echo ""
    echo "  1) OpenAI (text-embedding-3-small)    — requires API key"
    echo "  2) Ollama (nomic-embed-text)           — local, no API key"
    echo ""
    read -rp "Select provider [1]: " embed_choice
    embed_choice=${embed_choice:-1}

    case $embed_choice in
        1)
            sed -i.bak 's/^EMBED_PROVIDER=.*/EMBED_PROVIDER=openai/' .env
            sed -i.bak 's/^EMBED_MODEL_ID=.*/EMBED_MODEL_ID=text-embedding-3-small/' .env
            if [[ "$llm_choice" != "2" ]]; then
                read -rp "  OpenAI API key (for embeddings): " embed_key
                sed -i.bak "s/^# OPENAI_API_KEY=.*/OPENAI_API_KEY=${embed_key}/" .env
            fi
            ;;
        2)
            sed -i.bak 's/^EMBED_PROVIDER=.*/EMBED_PROVIDER=ollama/' .env
            sed -i.bak 's/^EMBED_MODEL_ID=.*/EMBED_MODEL_ID=nomic-embed-text/' .env
            sed -i.bak 's/^# OLLAMA_BASE_URL=.*/OLLAMA_BASE_URL=http:\/\/host.docker.internal:11434/' .env
            ;;
    esac
else
    echo ""
    echo -e "${BOLD}Step 2/3: Embedding Provider${NC} — auto-configured with LLM provider"
fi

# ── Document Source ───────────────────────────────────────────────────────

echo ""
echo -e "${BOLD}Step 3/3: Document Source${NC}"
echo ""
echo "  1) Sample docs (included — try DocBrain immediately)"
echo "  2) Local directory"
echo "  3) Confluence"
echo "  4) GitHub repository"
echo ""
read -rp "Select source [1]: " source_choice
source_choice=${source_choice:-1}

case $source_choice in
    1)
        # defaults are fine
        ;;
    2)
        read -rp "  Absolute path to docs directory: " docs_path
        sed -i.bak "s|^LOCAL_DOCS_PATH=.*|LOCAL_DOCS_PATH=${docs_path}|" .env
        ;;
    3)
        echo ""
        read -rp "  Confluence URL (e.g. https://yourco.atlassian.net/wiki): " conf_url
        read -rp "  Email: " conf_email
        read -rp "  API token: " conf_token
        read -rp "  Space keys (comma-separated): " conf_spaces
        # Write source credentials to config/local.yaml (gitignored), not .env
        cat >> config/local.yaml << CONFYAML

ingest:
  ingest_sources: confluence

confluence:
  base_url: ${conf_url}
  user_email: ${conf_email}
  api_token: ${conf_token}
  space_keys: ${conf_spaces}
CONFYAML
        echo ""
        echo -e "  ${GREEN}Confluence settings written to config/local.yaml (gitignored).${NC}"
        ;;
    4)
        echo ""
        read -rp "  Repository URL: " gh_url
        read -rp "  Token (optional, enter to skip): " gh_token
        read -rp "  Branch [main]: " gh_branch
        gh_branch=${gh_branch:-main}
        # Write source credentials to config/local.yaml (gitignored), not .env
        cat >> config/local.yaml << GHYAML

ingest:
  ingest_sources: github

github:
  repo_url: ${gh_url}
  branch: ${gh_branch}
GHYAML
        if [[ -n "$gh_token" ]]; then
            cat >> config/local.yaml << GHTOKENYAML
  token: ${gh_token}
GHTOKENYAML
        fi
        echo ""
        echo -e "  ${GREEN}GitHub settings written to config/local.yaml (gitignored).${NC}"
        ;;
esac

# Clean up sed backup files
rm -f .env.bak

echo ""
echo -e "${GREEN}Configuration saved.${NC}"
echo ""
echo "Starting DocBrain..."
echo ""

docker compose up -d

fi  # end SKIP_SETUP

# ── Wait for services to be ready ────────────────────────────────────────

echo ""
echo -n "Waiting for PostgreSQL"
for i in $(seq 1 30); do
    if docker compose exec -T postgres pg_isready -U docbrain > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

echo -n "Waiting for OpenSearch"
for i in $(seq 1 45); do
    if curl -sf http://localhost:9200/_cluster/health > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

echo -n "Waiting for server"
for i in $(seq 1 45); do
    if curl -sf http://localhost:3000/api/v1/health > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

# ── Auto-ingest sample docs on first run ─────────────────────────────────

INDEX_COUNT=$(curl -sf "http://localhost:9200/docbrain-chunks/_count" 2>/dev/null | grep -o '"count":[0-9]*' | cut -d: -f2 || echo "0")

if [[ "${INDEX_COUNT:-0}" -eq 0 ]]; then
    echo ""
    echo -e "${BOLD}Ingesting sample docs so you can try it immediately...${NC}"
    docker compose exec -T server docbrain-ingest 2>&1 | tail -5
    NEW_COUNT=$(curl -sf "http://localhost:9200/docbrain-chunks/_count" 2>/dev/null | grep -o '"count":[0-9]*' | cut -d: -f2 || echo "0")
    if [[ "${NEW_COUNT:-0}" -gt 0 ]]; then
        echo -e "${GREEN}Ingested sample docs (${NEW_COUNT} chunks). You can ask questions now!${NC}"
    fi
fi

# ── Bootstrap admin key ──────────────────────────────────────────────────

BOOTSTRAP_KEY=""
if docker compose exec -T server test -f /app/admin-bootstrap-key.txt 2>/dev/null; then
    BOOTSTRAP_KEY=$(docker compose exec -T server cat /app/admin-bootstrap-key.txt 2>/dev/null | grep "^Key:" | cut -d' ' -f2 || true)
fi

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║          DocBrain is running!                ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  Web UI       ${BLUE}http://localhost:3001${NC}"
echo -e "  API Server   ${BLUE}http://localhost:3000${NC}"
echo -e "  Health       ${BLUE}http://localhost:3000/api/v1/health${NC}"
echo ""

if [[ -n "$BOOTSTRAP_KEY" ]]; then
    echo -e "  ${BOLD}Admin API key:${NC} ${GREEN}${BOOTSTRAP_KEY}${NC}"
    echo ""
    echo "  This key expires in 120 minutes. Create a permanent admin account:"
    echo ""
    echo "    curl -X POST http://localhost:3000/api/v1/admin/users \\"
    echo "      -H 'X-API-Key: ${BOOTSTRAP_KEY}' \\"
    echo "      -H 'Content-Type: application/json' \\"
    echo "      -d '{\"email\":\"admin@example.com\",\"password\":\"yourpassword\",\"display_name\":\"Admin\",\"role\":\"admin\"}'"
    echo ""
else
    echo "  Retrieve your admin API key:"
    echo "    docker compose exec server cat /app/admin-bootstrap-key.txt"
    echo ""
fi

echo -e "  ${BOLD}Next steps:${NC}"
echo "    1. Create an admin account (use the key above)"
echo "    2. Open http://localhost:3001 and sign in"
echo "    3. Ask a question to verify everything works"
echo ""
echo "  To connect a document source, edit config/local.yaml (gitignored):"
echo "    — put source credentials (Confluence token, GitHub token, etc.) there"
echo "    — put infrastructure secrets (DATABASE_URL, API keys) in .env"
echo ""
echo "  To ingest your own docs:"
echo "    docker compose exec server docbrain-ingest"
echo ""
echo "  To stop:  docker compose down"
echo "  To logs:  docker compose logs -f"
echo ""
