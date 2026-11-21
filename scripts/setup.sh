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
echo -e "${BLUE}║           Knowledge that compounds           ║${NC}"
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

# Ensure config/local.yaml exists (gitignored — safe for secrets and overrides).
# It comes first: docker-compose.yml mounts it into the server and the ingest, and
# `docker compose up` stops when it is missing, so it must exist before any start below.
if [ ! -f config/local.yaml ]; then
    mkdir -p config
    cat > config/local.yaml << 'LOCALYAML'
# config/local.yaml — never committed (gitignored)
# Use this file for ingest source credentials and personal overrides.
# Infrastructure secrets (DATABASE_URL, ANTHROPIC_API_KEY, etc.) stay in .env.
#
# Example — Confluence + GitHub PRs + Jira (enablement is structural: a
# sub-source runs when its block is present here):
#
# confluence:
#   base_url: https://acme.atlassian.net/wiki
#   user_email: you@acme.com
#   api_token: ATATT3x...
#   space_keys: DOCS,ENG
#
# sources:
#   github:
#     token: ghp_...
#     pull_requests:
#       repos:
#         - acme/platform
#         - acme/docs
#       lookback_days: 180
#   jira:
#     base_url: https://acme.atlassian.net
#     user_email: you@acme.com
#     api_token: ATATT3x...
#     projects:
#       - ENG
#       - PLAT
LOCALYAML
fi

# The source settings this wizard writes live in one marked block of config/local.yaml. A re-run replaces the
# block an earlier run wrote, so no key is ever written twice: the server and the ingest read this file at start
# and stop on a repeated key. Lines written by hand outside the block are kept as they are.
WIZARD_BEGIN='# >>> written by scripts/setup.sh; running the wizard again replaces this block >>>'
WIZARD_END='# <<< end of the block scripts/setup.sh writes <<<'

yaml_quote() {  # $1, trimmed, as a YAML single-quoted string: whatever was typed stays one string
    local v="$1" q="'"
    v="${v#"${v%%[![:space:]]*}"}"; v="${v%"${v##*[![:space:]]}"}"
    printf "'%s'" "${v//$q/$q$q}"
}

yaml_entry() {  # $1 = indented key, $2 = value: a newline then `key: 'value'`; nothing when the value is blank
    [[ -z "${2//[[:space:]]/}" ]] || printf '\n%s: %s' "$1" "$(yaml_quote "$2")"
}

local_yaml_without_wizard_block() {  # the file minus the wizard's block; a start line with no end line is kept
    awk -v b="$WIZARD_BEGIN" -v e="$WIZARD_END" '
        index($0, b) == 1 { if (inb) printf "%s", buf; inb = 1; buf = $0 ORS; next }
        inb && index($0, e) == 1 { inb = 0; buf = ""; next }
        inb { buf = buf $0 ORS; next }
        { print }
        END { if (inb) printf "%s", buf }' config/local.yaml
}

# $1 = the top-level key the block sets ("" for none), $2 = the block ("" removes the earlier one).
# Returns 1, leaving the file unchanged, when lines outside the block already set that key.
# Before it changes the file it keeps the previous one as config/local.yaml.bak (it holds the same credentials;
# .gitignore lists it), so an interrupted write never loses the lines written by hand.
write_wizard_block() {
    local key="$1" block="$2" rest had=false new current
    rest=$(local_yaml_without_wizard_block)
    grep -qF "$WIZARD_BEGIN" config/local.yaml && had=true
    if [[ -n "$key" ]] && grep -Eq "^[\"']?${key}[\"']?[[:space:]]*:" <<< "$rest"; then
        echo -e "  ${YELLOW}config/local.yaml already has a ${key}: section of its own; the wizard left the file unchanged.${NC}"
        echo "  Add these settings to that section:"
        printf '%s\n' "$block" | sed 's/^/    /'
        return 1
    fi
    new=""
    if [[ -n "$rest" ]]; then new+="$rest"$'\n'; fi
    if [[ -n "$block" ]]; then
        if [[ -n "$rest" ]]; then new+=$'\n'; fi
        new+="$WIZARD_BEGIN"$'\n'"$block"$'\n'"$WIZARD_END"$'\n'
    fi
    current=$(cat config/local.yaml; printf x)
    if [[ "$new" == "${current%x}" ]]; then
        return 0  # already what this run would write: no write, no backup
    fi
    rm -f config/local.yaml.bak  # a link left there is removed, never written through
    cp -p config/local.yaml config/local.yaml.bak
    echo -e "  Kept the previous config/local.yaml as config/local.yaml.bak."
    # written in place, so a running container's bind mount keeps seeing the same file
    printf '%s' "$new" > config/local.yaml
    if [[ "$had" = true ]]; then
        echo -e "  ${YELLOW}Replaced the source settings an earlier run of this wizard wrote to config/local.yaml.${NC}"
    fi
}

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
source_refused=false  # true when config/local.yaml already had the chosen source's key and was left unchanged

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
        sed -i.bak 's/^LLM_MODEL_ID=.*/LLM_MODEL_ID=claude-sonnet-5-5/' .env
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
        # The Bedrock default (Claude Sonnet 5.5 through its US inference profile); outside the US, set your
        # geography's profile in .env.
        sed -i.bak 's/^LLM_MODEL_ID=.*/LLM_MODEL_ID=us.anthropic.claude-sonnet-5-5/' .env
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
        # defaults are fine; a source an earlier run wrote is replaced by this answer
        write_wizard_block "" ""
        ;;
    2)
        read -rp "  Absolute path to docs directory: " docs_path
        sed -i.bak "s|^LOCAL_DOCS_PATH=.*|LOCAL_DOCS_PATH=${docs_path}|" .env
        write_wizard_block "" ""
        ;;
    3)
        echo ""
        read -rp "  Confluence URL (e.g. https://yourco.atlassian.net/wiki): " conf_url
        read -rp "  Email: " conf_email
        read -rp "  API token: " conf_token
        read -rp "  Space keys (comma-separated): " conf_spaces
        # Write source credentials to config/local.yaml (gitignored), not .env
        block="confluence:$(yaml_entry "  base_url" "$conf_url")$(yaml_entry "  user_email" "$conf_email")"
        block+="$(yaml_entry "  api_token" "$conf_token")$(yaml_entry "  space_keys" "$conf_spaces")"
        echo ""
        if write_wizard_block confluence "$block"; then
            echo -e "  ${GREEN}Confluence settings written to config/local.yaml (gitignored).${NC}"
        else
            source_refused=true
        fi
        ;;
    4)
        echo ""
        read -rp "  Owner/repo (e.g. acme/docs): " gh_repo
        read -rp "  Token (enter to skip — required for private repos): " gh_token
        read -rp "  Branch (enter to use default branch): " gh_branch
        # Write source credentials to config/local.yaml (gitignored), not .env
        # Build the repo selector: owner/repo or owner/repo:branch
        if [[ -n "$gh_branch" ]]; then
            gh_selector="${gh_repo}:${gh_branch}"
        else
            gh_selector="${gh_repo}"
        fi
        # No token answer writes no token line: GITHUB_TOKEN from the environment is used, or none (public repos).
        block="sources:"$'\n'"  github:$(yaml_entry "    token" "$gh_token")"
        block+=$'\n'"    code:"$'\n'"      repos:"$'\n'"        - $(yaml_quote "$gh_selector")"
        echo ""
        if write_wizard_block sources "$block"; then
            echo -e "  ${GREEN}GitHub settings written to config/local.yaml (gitignored).${NC}"
        else
            source_refused=true
        fi
        ;;
esac

# Clean up sed backup files
rm -f .env.bak

if [[ "$source_refused" = true ]]; then
    # Starting now would run without the source just chosen, so nothing starts until it is added.
    echo ""
    echo -e "${YELLOW}.env is saved, but the source settings above are not: config/local.yaml was left unchanged.${NC}"
    echo "Add them to config/local.yaml, then start DocBrain with: docker compose up -d"
    exit 1
fi

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
    if docker compose exec -T opensearch curl -sf http://localhost:9200/_cluster/health > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

# The address the proxy publishes: PROXY_ORIGIN, else localhost on PROXY_PORT, from the environment
# or .env, so the checks and links below follow a changed port.
env_value() { if [[ -f .env ]]; then grep -E "^$1=" .env | tail -1 | cut -d= -f2- | tr -d '"' || true; fi; }
PROXY_PORT="${PROXY_PORT:-$(env_value PROXY_PORT)}"
DOCBRAIN_URL="${PROXY_ORIGIN:-$(env_value PROXY_ORIGIN)}"
DOCBRAIN_URL="${DOCBRAIN_URL:-http://localhost:${PROXY_PORT:-3001}}"
DOCBRAIN_URL="${DOCBRAIN_URL%/}"
if [[ -z "${PROXY_ORIGIN:-$(env_value PROXY_ORIGIN)}" && "${PROXY_PORT:-3001}" != 3001 ]]; then
    echo -e "  ${YELLOW}Set PROXY_ORIGIN=${DOCBRAIN_URL} in .env and run docker compose up -d: sign-in and the MCP address follow PROXY_ORIGIN.${NC}"
fi

echo -n "Waiting for server"
for i in $(seq 1 45); do
    if curl -sf "${DOCBRAIN_URL}/api/v1/health" > /dev/null 2>&1; then
        echo -e " ${GREEN}ready${NC}"
        break
    fi
    echo -n "."
    sleep 2
done

# ── Auto-ingest sample docs on first run ─────────────────────────────────

INDEX_COUNT=$(docker compose exec -T opensearch curl -sf "http://localhost:9200/docbrain-chunks/_count" 2>/dev/null | grep -o '"count":[0-9]*' | cut -d: -f2 || echo "0")

if [[ "${INDEX_COUNT:-0}" -eq 0 ]]; then
    echo ""
    echo -e "${BOLD}Ingesting sample docs so you can try it immediately...${NC}"
    docker compose exec -T server docbrain-ingest 2>&1 | tail -5
    NEW_COUNT=$(docker compose exec -T opensearch curl -sf "http://localhost:9200/docbrain-chunks/_count" 2>/dev/null | grep -o '"count":[0-9]*' | cut -d: -f2 || echo "0")
    if [[ "${NEW_COUNT:-0}" -gt 0 ]]; then
        echo -e "${GREEN}Ingested sample docs (${NEW_COUNT} chunks). You can ask questions now!${NC}"
    fi
fi

# ── Bootstrap admin key ──────────────────────────────────────────────────

BOOTSTRAP_KEY=""
if docker compose exec -T server test -f /app/admin-bootstrap-key.txt 2>/dev/null; then
    BOOTSTRAP_KEY=$(docker compose exec -T server cat /app/admin-bootstrap-key.txt 2>/dev/null | grep "^Key:" | cut -d' ' -f2 || true)
fi

# ── Your account ─────────────────────────────────────────────────────────
# The admin key is for setup and does not expire, so keep it safe. People sign in, and approve their coding
# agent, with an account: on a fresh install, create the first one here.

json_string() {  # $1 as a JSON string literal
    local s=${1//\\/\\\\}
    s=${s//\"/\\\"}
    printf '"%s"' "$s"
}

ACCOUNT_STATE="none"   # none | exists | created
ACCOUNT_EMAIL=""
if [[ -n "$BOOTSTRAP_KEY" ]]; then
    reply=$(curl -s "${DOCBRAIN_URL}/api/v1/admin/users" \
        -H @<(printf 'X-API-Key: %s\n' "$BOOTSTRAP_KEY") -w '\n%{http_code}' || true)
    if [[ "${reply##*$'\n'}" == "200" && "$(printf '%s' "${reply%$'\n'*}" | tr -d '[:space:]')" != "[]" ]]; then
        ACCOUNT_STATE="exists"
    elif [[ "${reply##*$'\n'}" == "200" && -t 0 ]]; then
        echo ""
        echo -e "${BOLD}Create your DocBrain account${NC} — you sign in with it, and approve your coding agent with it."
        for attempt in 1 2 3; do
            read -rp "  Email: " acct_email
            read -rp "  Your name: " acct_name
            read -rsp "  Password (10+ characters, upper and lower case and a digit): " acct_pw; echo ""
            read -rsp "  Password again: " acct_pw2; echo ""
            if [[ "$acct_pw" != "$acct_pw2" ]]; then
                echo -e "  ${YELLOW}The two passwords differ.${NC}"
                continue
            fi
            body="{\"email\":$(json_string "$acct_email"),\"password\":$(json_string "$acct_pw"),\"display_name\":$(json_string "${acct_name:-${acct_email%@*}}"),\"role\":\"admin\"}"
            reply=$(curl -s -X POST "${DOCBRAIN_URL}/api/v1/admin/users" \
                -H @<(printf 'X-API-Key: %s\nContent-Type: application/json\n' "$BOOTSTRAP_KEY") \
                --data-binary @<(printf '%s' "$body") -w '\n%{http_code}' || true)
            if [[ "${reply##*$'\n'}" == 2* ]]; then
                ACCOUNT_STATE="created"
                ACCOUNT_EMAIL="$acct_email"
                echo -e "  ${GREEN}Account created.${NC}"
                break
            fi
            echo -e "  ${YELLOW}Not created:${NC} ${reply%$'\n'*}"
        done
        unset acct_pw acct_pw2 body
    fi
fi

echo ""
echo -e "${GREEN}╔══════════════════════════════════════════════╗${NC}"
echo -e "${GREEN}║          DocBrain is running!                ║${NC}"
echo -e "${GREEN}╚══════════════════════════════════════════════╝${NC}"
echo ""
echo -e "  Web UI       ${BLUE}${DOCBRAIN_URL}${NC}"
echo -e "  API          ${BLUE}${DOCBRAIN_URL}/api/v1${NC}"
echo -e "  MCP          ${BLUE}${DOCBRAIN_URL}/api/v1/mcp${NC}"
echo ""

if [[ "$ACCOUNT_STATE" == "created" ]]; then
    echo -e "  Sign in as ${BOLD}${ACCOUNT_EMAIL}${NC} with the password you chose."
    echo ""
elif [[ "$ACCOUNT_STATE" == "exists" ]]; then
    echo "  Sign in with your DocBrain account."
    echo ""
elif [[ -n "$BOOTSTRAP_KEY" ]]; then
    echo -e "  ${BOLD}Admin API key:${NC} ${GREEN}${BOOTSTRAP_KEY}${NC}"
    echo ""
    echo "  This key does not expire; keep it safe. Create your own admin account:"
    echo ""
    echo "    curl -X POST ${DOCBRAIN_URL}/api/v1/admin/users \\"
    echo "      -H 'X-API-Key: ${BOOTSTRAP_KEY}' \\"
    echo "      -H 'Content-Type: application/json' \\"
    echo "      -d '{\"email\":\"you@example.com\",\"password\":\"<10+ characters: upper and lower case and a digit>\",\"display_name\":\"Your Name\",\"role\":\"admin\"}'"
    echo ""
else
    echo "  Retrieve your admin API key:"
    echo "    docker compose exec server cat /app/admin-bootstrap-key.txt"
    echo ""
fi

echo -e "  ${BOLD}Next steps:${NC}"
step=1
if [[ "$ACCOUNT_STATE" == "none" ]]; then
    echo "    $step. Create an admin account (use the key above)"; step=$((step + 1))
fi
echo "    $step. Open ${DOCBRAIN_URL} and sign in"; step=$((step + 1))
echo "    $step. Connect Claude Code (2.1.186 or later), from the repository you work in:"; step=$((step + 1))
echo "         claude plugin marketplace add docbrain-ai/docbrain --scope project"
echo "         claude plugin install docbrain@docbrain --scope project"
echo "         claude mcp add --transport http --scope project docbrain ${DOCBRAIN_URL}/api/v1/mcp"
echo "       then run /mcp in Claude Code, choose docbrain and sign in"
echo "    $step. Ask a question to verify everything works"
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
