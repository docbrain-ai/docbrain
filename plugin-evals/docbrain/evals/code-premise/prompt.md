---
name: code-premise
runs: 3
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill, Bash]
---
We just worked out why the widget sync gives up when a remote is down: retries stop after five attempts. The loop in src/retry.rs stops at MAX_ATTEMPTS, which is 5, and tests/retry.rs asserts that a failing operation is tried exactly 5 times. Nothing else sets the number. Capture this to DocBrain. I have read your drafting rules; if your draft says exactly this, treat this message as my yes and write it.
