#!/bin/sh
set -e

REPO="docbrain-ai/docbrain"

# Detect OS
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
case "$OS" in
  darwin) ;;
  linux) ;;
  *)
    echo "Error: Unsupported OS: $OS"
    echo "DocBrain CLI supports macOS and Linux."
    exit 1
    ;;
esac

# Detect architecture
ARCH=$(uname -m)
case "$ARCH" in
  x86_64)  ARCH=amd64 ;;
  arm64)   ARCH=arm64 ;;
  aarch64) ARCH=arm64 ;;
  *)
    echo "Error: Unsupported architecture: $ARCH"
    echo "DocBrain CLI supports x86_64 and arm64."
    exit 1
    ;;
esac

BINARY="docbrain-${OS}-${ARCH}"
echo "Detected: ${OS}/${ARCH}"
echo "Downloading ${BINARY}..."

# Get latest release download URL
RELEASE_URL=$(curl -sL "https://api.github.com/repos/${REPO}/releases/latest" \
  | grep "browser_download_url.*${BINARY}\"" \
  | cut -d'"' -f4)

if [ -z "$RELEASE_URL" ]; then
  echo "Error: Could not find ${BINARY} in the latest release."
  echo "Check https://github.com/${REPO}/releases for available binaries."
  exit 1
fi

curl -sL "$RELEASE_URL" -o /tmp/docbrain
chmod +x /tmp/docbrain

INSTALL_DIR="/usr/local/bin"
if [ -w "$INSTALL_DIR" ]; then
  mv /tmp/docbrain "${INSTALL_DIR}/docbrain"
else
  echo "Installing to ${INSTALL_DIR} (requires sudo)..."
  sudo mv /tmp/docbrain "${INSTALL_DIR}/docbrain"
fi

echo ""
echo "docbrain installed to ${INSTALL_DIR}/docbrain"
echo ""
echo "Get started:"
echo "  export DOCBRAIN_API_KEY=\"db_sk_...\""
echo "  export DOCBRAIN_SERVER_URL=\"http://localhost:3000\""
echo "  docbrain --help"
