---
name: commit-why
runs: 3
max_turns: 10
allowed_tools: [Read, Glob, Grep, Skill]
---
I am about to commit a one-line change to services/payments/webhook.py: MAX_ATTEMPTS goes from 3 to 1. The message will say "payments: one webhook attempt". The reason is that the provider retries on its side and our retries doubled every failed charge attempt during the incident on the 9th; one attempt, then the provider's schedule. Record why in DocBrain — yes, write it.
