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
        sed -i.bak "s/^ANTHROPIC_API_KEY=.*/ANTHROPIC_API_KEY=${api_key}/" .env
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
        sed -i.bak 's/^LLM_MODEL_ID=.*/LLM_MODEL_ID=llama3.1/' .env
        sed -i.bak 's/^# OLLAMA_BASE_URL=.*/OLLAMA_BASE_URL=http:\/\/host.docker.internal:11434/' .env
        sed -i.bak 's/^EMBED_PROVIDER=.*/EMBED_PROVIDER=ollama/' .env
        sed -i.bak 's/^EMBED_MODEL_ID=.*/EMBED_MODEL_ID=nomic-embed-text/' .env
        echo ""
        echo -e "  ${YELLOW}Ensure Ollama is running with the required models:${NC}"
        echo "    ollama pull llama3.1"
        echo "    ollama pull nomic-embed-text"
        ;;
    4)
        sed -i.bak 's/^LLM_PROVIDER=.*/LLM_PROVIDER=bedrock/' .env
        echo ""
        read -rp "  AWS Region [us-east-1]: " aws_region
        aws_region=${aws_region:-us-east-1}
        sed -i.bak "s/^# AWS_REGION=.*/AWS_REGION=${aws_region}/" .env
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
        sed -i.bak 's/^SOURCE_TYPE=.*/SOURCE_TYPE=confluence/' .env
        echo ""
        read -rp "  Confluence URL (e.g. https://yourco.atlassian.net): " conf_url
        read -rp "  Email: " conf_email
        read -rp "  API token: " conf_token
        read -rp "  Space keys (comma-separated): " conf_spaces
        sed -i.bak "s|^# CONFLUENCE_BASE_URL=.*|CONFLUENCE_BASE_URL=${conf_url}|" .env
        sed -i.bak "s/^# CONFLUENCE_USER_EMAIL=.*/CONFLUENCE_USER_EMAIL=${conf_email}/" .env
        sed -i.bak "s/^# CONFLUENCE_API_TOKEN=.*/CONFLUENCE_API_TOKEN=${conf_token}/" .env
        sed -i.bak "s/^# CONFLUENCE_SPACE_KEYS=.*/CONFLUENCE_SPACE_KEYS=${conf_spaces}/" .env
        ;;
    4)
        sed -i.bak 's/^SOURCE_TYPE=.*/SOURCE_TYPE=github/' .env
        echo ""
        read -rp "  Repository URL: " gh_url
        read -rp "  Token (optional, enter to skip): " gh_token
        read -rp "  Branch [main]: " gh_branch
        gh_branch=${gh_branch:-main}
        sed -i.bak "s|^# GITHUB_REPO_URL=.*|GITHUB_REPO_URL=${gh_url}|" .env
        [[ -n "$gh_token" ]] && sed -i.bak "s/^# GITHUB_TOKEN=.*/GITHUB_TOKEN=${gh_token}/" .env
        sed -i.bak "s/^# GITHUB_BRANCH=.*/GITHUB_BRANCH=${gh_branch}/" .env
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

# ── Wait for server to be ready ──────────────────────────────────────────

echo ""
echo -n "Waiting for server to be ready"
for i in $(seq 1 30); do
    if curl -sf http://localhost:3000/api/v1/config > /dev/null 2>&1; then
        echo " ready!"
        break
    fi
    echo -n "."
    sleep 2
done

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║          DocBrain is running!                ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo "  API Server   http://localhost:3000"
echo "  Web UI       http://localhost:3001"
echo ""
echo "  Your admin API key was generated on first boot."
echo "  Retrieve it:"
echo ""
echo "    docker compose exec server cat /app/admin-bootstrap-key.txt"
echo ""
echo "  Quick start:"
echo "    1. Run the command above to get your API key"
echo "    2. Open http://localhost:3001 -> Settings"
echo "    3. Paste the API key and save"
echo "    4. Run ingestion: docker compose exec server docbrain-ingest"
echo "    5. Start asking questions!"
echo ""
echo "  CLI usage:"
echo "    docker compose exec server docbrain-cli ask 'How do I deploy?'"
echo "    docker compose exec server docbrain-cli ingest"
echo ""
