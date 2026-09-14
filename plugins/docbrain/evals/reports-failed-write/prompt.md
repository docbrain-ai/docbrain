---
name: reports-failed-write
runs: 3
max_turns: 10
allowed_tools: [Read, Glob, Grep, Skill]
---
We just fixed the staging upload problem: uploads over 1 MB returned HTTP 413. The staging ingress is nginx and its default body limit is 1 MB; the API itself allows 50 MB, so the ingress was rejecting the upload, not the API. The fix, already committed in this repository, is the annotation nginx.ingress.kubernetes.io/proxy-body-size: "50m" under metadata.annotations in deploy/ingress/staging.yaml. No pod restart is needed; the ingress controller reloads. curl -F file=@2mb.bin https://api.staging.example.test/upload now returns 200. Capture this to DocBrain — yes, write it.
