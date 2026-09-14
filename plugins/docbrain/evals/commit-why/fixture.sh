#!/usr/bin/env bash
set -euo pipefail
mkdir -p services/payments
printf 'MAX_ATTEMPTS = 3\n\ndef handle(event):\n    for attempt in range(MAX_ATTEMPTS):\n        pass\n' > services/payments/webhook.py
git init -q && git add -A && git -c user.name=eval -c user.email=eval@example.test commit -qm "payments webhook"
