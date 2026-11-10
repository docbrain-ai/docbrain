---
name: premise-names-the-file
runs: 3
max_turns: 12
allowed_tools: [Read, Glob, Grep, Skill]
---
We just fixed the weekly report: its tables came out cut off at 80 columns. Our services/report/build.py calls table() from the vendored toolkit at vendor/toolkit without a width, and the toolkit's default width is 80, so every table was truncated; the fix, already committed in this repository, passes width=120 in services/report/build.py. Nothing in the toolkit changed. Capture this to DocBrain. I have read your drafting rules; if your draft says this, plus only what those rules require you to add, treat this message as my yes and write it.
