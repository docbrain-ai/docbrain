#!/usr/bin/env bash
set -euo pipefail

# Build DocBrain docs site and deploy to the website repo
# Usage: ./scripts/build-docs.sh

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PUBLIC_REPO="$(dirname "$SCRIPT_DIR")"
WEBSITE_REPO="${WEBSITE_REPO:-$(dirname "$PUBLIC_REPO")/docbrain-ai.github.io}"
BUILD_DIR="/tmp/docbrain-docs-site"

if [ ! -d "$WEBSITE_REPO" ]; then
  echo "Error: Website repo not found at $WEBSITE_REPO"
  echo "Set WEBSITE_REPO env var to override."
  exit 1
fi

echo "Building docs from: $PUBLIC_REPO"
echo "Deploying to:       $WEBSITE_REPO/docs/"

# Build
cd "$PUBLIC_REPO"
python3 -m mkdocs build --site-dir "$BUILD_DIR" --clean

# Deploy
rm -rf "$WEBSITE_REPO/docs"
cp -r "$BUILD_DIR" "$WEBSITE_REPO/docs"

echo ""
echo "Done. Built $(find "$WEBSITE_REPO/docs" -name '*.html' | wc -l | tr -d ' ') pages."
echo ""
echo "Next steps:"
echo "  cd $WEBSITE_REPO"
echo "  git add docs/ && git commit -m 'docs: rebuild docs site'"
echo "  git push"
