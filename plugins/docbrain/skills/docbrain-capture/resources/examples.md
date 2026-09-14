# Three finished captures

Each is what the person saw before saying yes, and what was sent.

## A procedure that took digging

```
Type:   procedure
File:   deploy/ingress/staging.yaml   Lines: 12-19

Uploads over 1 MB to the staging API fail with HTTP 413 until the ingress allows a larger body.

The staging ingress is nginx; its default body limit is 1 MB and the API's own limit is 50 MB, so
the ingress, not the API, is what rejects the upload.

1. Set the annotation nginx.ingress.kubernetes.io/proxy-body-size: "50m" on the API ingress.
2. Apply with the usual deploy; no pod restart is needed — the ingress controller reloads.
3. Verify: curl -F file=@2mb.bin https://staging.example.test/upload returns 200.

Premises:
  - deploy/ingress/staging.yaml
```

Sent as `docbrain_annotate` with `fragment_type: "procedure"`, `file_path: "deploy/ingress/staging.yaml"`,
`line_range: "12-19"`, the eight lines at that range as `code_snippet`, the three paragraphs as `annotation`, and one
`path` premise. Reported back as indexed.

## A decision and what it rejected

```
Type:   decision
File:   crates/docbrain-server/src/pool.rs   Lines: 40-58

The API keeps two Postgres pools: an application-role pool for request handlers and a service pool
for background work.

One pool would let a background job run with the application role's restricted grants, or a
request handler run with service privileges; both were rejected. A single role with broader grants
was rejected too — the restricted role is what makes a compromised key harmless to the schema.

Handlers take state.app_db; schedulers, rollups and engines take the service pool. Mixing them is a
security defect, not a style choice.

Premises:
  - crates/docbrain-server/src/pool.rs
```

## A caveat

```
Type:   caveat
File:   scripts/deploy.sh   Lines: 88-96

The deploy script "succeeds" when the image tag it was told to write is absent from the override file.

Its sed replaces an old tag with a new one; if the old tag is not in the file, nothing changes,
compose recreates the container on the old image, and the script still prints DEPLOYED.

Check that the new tag is present in the file after the sed and before compose runs; refuse
otherwise.

Premises:
  - scripts/deploy.sh
```

## The why of a commit

The change is small; the message says what, not why. Captured at the commit, so the reasoning travels
with the diff.

```
Intent:   The retry loop's cap moved from 3 to 1 for the payments webhook because the provider
          retries on its side; our retries doubled every failed charge attempt during the incident
          on the 9th. One attempt, then the provider's schedule.
Files:    services/payments/webhook.py
Message:  payments: one webhook attempt, the provider retries
Diff:     MAX_ATTEMPTS 3 → 1 in the webhook handler; the backoff table removed.
```

Sent as `docbrain_commit_capture` with `intent`, `file_paths: ["services/payments/webhook.py"]`,
`commit_message` and `diff_summary`. Reported back as indexed, anchored to the file.
