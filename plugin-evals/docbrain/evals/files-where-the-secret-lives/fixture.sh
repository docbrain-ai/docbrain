#!/usr/bin/env bash
set -euo pipefail
mkdir -p services/notifier
cat > services/notifier/config.yaml <<'Y'
smtp:
  port: 587
  username: notifier
  password_from: vault notifier/relay smtp_password
alerts:
  token_from: vault notifier/relay alerts_token
Y
git init -q && git add -A && git -c user.name=eval -c user.email=eval@example.test commit -qm "notifier reads its credentials from the team vault"
