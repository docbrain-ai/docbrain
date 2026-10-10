#!/usr/bin/env bash
set -euo pipefail
mkdir -p services/notifier
cat > services/notifier/config.yaml <<'Y'
smtp:
  port: 587
  username: notifier
  password: ${NOTIFIER_SMTP_PASSWORD}
alerts:
  token: ${NOTIFIER_ALERTS_TOKEN}
Y
git init -q && git add -A && git -c user.name=eval -c user.email=eval@example.test commit -qm "notifier reads its credentials from the environment"
