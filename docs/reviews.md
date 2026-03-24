# Review Workflows

DocBrain's review system ensures that no documentation — whether AI-generated, composed from fragments, or manually written — goes live without human oversight. Review workflows are configurable per space with multi-stage approval pipelines.

## Overview

A review workflow defines the stages a document draft must pass through before publication. Each stage has assigned reviewers and required actions (approve, request changes, or reject).

```
Draft Created → Stage 1: SME Review → Stage 2: Writing Review → Stage 3: Publish Approval → Published
                    │                      │                          │
                    ├── Approved ──→        ├── Approved ──→           ├── Approved ──→ Published
                    ├── Changes ──→ Author  ├── Changes ──→ Author    └── Rejected ──→ Archived
                    └── Rejected ──→ Closed └── Rejected ──→ Closed
```

## Creating a Workflow

Workflows are defined per space. A space without a workflow will use the default single-stage approval.

**API:**
```bash
POST /api/v1/reviews/workflows
{
  "name": "Platform Docs Review",
  "space": "PLATFORM",
  "stages": [
    {
      "name": "SME Review",
      "description": "Subject matter expert validates technical accuracy",
      "required_approvals": 1,
      "reviewer_roles": ["editor", "analyst", "admin"]
    },
    {
      "name": "Writing Review",
      "description": "Technical writer checks clarity and style",
      "required_approvals": 1,
      "reviewer_roles": ["editor", "admin"]
    }
  ]
}
```

**Managing workflows:**
```bash
# List all workflows
GET /api/v1/reviews/workflows

# Update a workflow
PUT /api/v1/reviews/workflows/{id}

# Delete a workflow
DELETE /api/v1/reviews/workflows/{id}
```

## Submitting a Draft for Review

Any draft — from autopilot, fragment composition, or manual creation — can enter the review pipeline:

```bash
POST /api/v1/autopilot/drafts/{draft_id}/submit-for-review
```

This advances the draft from `generated` status into the first stage of the space's review workflow.

## Reviewing a Draft

### Personal Review Queue

Every reviewer has a queue of drafts waiting for their action:

```bash
# Get my pending reviews
GET /api/v1/reviews/my-queue
```

The web UI shows this at **Govern → Reviews** with tabs for your queue and workflow management.

### Submitting a Review

```bash
POST /api/v1/reviews/drafts/{draft_id}/review
{
  "action": "approve",       // or "request_changes" or "reject"
  "comment": "Looks good, technical details are accurate."
}
```

**Actions:**
- **`approve`** — Advances the draft to the next stage (or publishes if it's the final stage)
- **`request_changes`** — Sends the draft back to the author with feedback
- **`reject`** — Closes the review. The draft can be reworked and resubmitted later.

### Skipping Review

Admins can bypass the review workflow when necessary:

```bash
POST /api/v1/reviews/drafts/{draft_id}/skip-review
```

### Checking Draft Status

```bash
# Get current stage and review history for a draft
GET /api/v1/reviews/drafts/{draft_id}/stage

# List all reviews for a draft
GET /api/v1/reviews/drafts/{draft_id}/reviews
```

## Comments

Reviewers can leave threaded comments on drafts for inline feedback:

```bash
# Add a comment
POST /api/v1/reviews/drafts/{draft_id}/comments
{
  "content": "This section needs a code example for the retry logic.",
  "section": "error-handling"
}

# List comments
GET /api/v1/reviews/drafts/{draft_id}/comments

# Update a comment
PUT /api/v1/reviews/comments/{comment_id}

# Resolve a comment (mark as addressed)
POST /api/v1/reviews/comments/{comment_id}/resolve
```

## Events

Review actions emit events to the event bus:

| Event | Trigger |
|---|---|
| `draft.submitted_for_review` | Draft enters review pipeline |
| `draft.review_submitted` | Reviewer approves, requests changes, or rejects |
| `draft.stage_advanced` | Draft moves to next workflow stage |
| `draft.published` | Draft passes all stages and is published |

These events trigger notifications to relevant reviewers and space owners. They can also be forwarded to external systems via webhooks.

## Configuration

Review workflows are managed entirely via the API and web UI — no config file changes needed. The review system is always enabled.

## Roles Required

| Action | Minimum Role |
|---|---|
| View review queue | `editor` |
| Submit a review | `editor` |
| Add/update comments | `editor` |
| Skip review (bypass) | `admin` |
| Create/update/delete workflows | `admin` |
| Submit draft for review | `editor` |
