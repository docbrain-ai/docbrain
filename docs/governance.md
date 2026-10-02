# Governance

Documentation without ownership decays. DocBrain's governance system makes ownership, accountability, and quality standards explicit — with automated enforcement.

## Overview

Governance in DocBrain has four pillars:

1. **Space Ownership** — Who is responsible for which knowledge areas
2. **Topic Stewardship** — Subject-matter experts for specific topics within spaces
3. **SLA Policies** — Deadlines for picking up and resolving gaps and for draft review (a freshness term is stored but no check reads it today)
4. **Breach Detection** — Automated scanning that surfaces violations before they become incidents

All governance features are accessible from the **Governance** page in the web UI (sidebar → Govern → Governance), which shows the Govern overview and rules/owners management via tab navigation.

---

## Spaces and Ownership

A **space** is a logical grouping of documentation — typically aligned to a team, product area, or domain (e.g., `PLATFORM`, `API`, `ONBOARDING`, `SRE`).

Every space should have:
- **Owners** — Accountable for the space's overall documentation health. Receive SLA breach notifications.
- **Maintainers** — Can approve drafts and manage content within the space.

### Managing Space Owners

**Web UI:** Governance → Rules & Owners tab → select a space → manage owners.

**API:**
```bash
# List spaces with ownership info
GET /api/v1/governance/spaces

# Add an owner to a space
POST /api/v1/governance/spaces/{space}/owners
{ "user_id": "uuid", "role": "owner" }

# Remove an owner
DELETE /api/v1/governance/spaces/{space}/owners/{user_id}
```

### Topic Stewards

A **topic steward** is a subject-matter expert responsible for a specific topic within a space. When a documentation gap is detected in their topic area, they're notified and can be assigned to resolve it.

```bash
# List stewards
GET /api/v1/governance/stewards

# Create a steward assignment
POST /api/v1/governance/stewards
{
  "user_id": "uuid",
  "space": "PLATFORM",
  "topic": "kubernetes-networking",
  "description": "Owns all K8s networking documentation"
}

# Remove a steward
DELETE /api/v1/governance/stewards/{id}
```

### My Governance

Every user can see their own governance responsibilities:

```bash
# Spaces I own
GET /api/v1/governance/my-spaces

# Topics I steward
GET /api/v1/governance/my-stewardships
```

---

## SLA Policies

SLA policies define deadlines for documentation activities. They can be set globally (default) or per-space.

### SLA Types

| SLA Type | What It Measures | Default |
|---|---|---|
| **Gap Acknowledgment** | Time from a gap opening until someone picks it up (it is assigned) | 24 hours |
| **Gap Resolution** | Time from gap detection to documentation being written | 7 days |
| **Draft Review** | Time from draft submission to review completion | 48 hours |
| **Freshness** | Stored with the policy; no check reads it today, so nothing is reported late for it | 90 days |

### Configuring SLAs

**Web UI:** Govern › SLAs shows what is late now. Rules & Owners tab lets you manage per-space policies.

**API:**
```bash
# List all SLA policies
GET /api/v1/governance/slas

# Set default SLA policy
PUT /api/v1/governance/slas/default
{
  "gap_acknowledgment_hours": 24,
  "gap_resolution_days": 7,
  "draft_review_hours": 48,
  "freshness_review_days": 90
}

# Set space-specific SLA (overrides default)
PUT /api/v1/governance/slas/{space}
{
  "gap_acknowledgment_hours": 12,
  "gap_resolution_days": 3,
  "draft_review_hours": 24,
  "freshness_review_days": 60
}

# Delete a space-specific policy (falls back to default)
DELETE /api/v1/governance/slas/{space}
```

### Breach Detection

DocBrain runs an automated SLA checker on a configurable interval (default: every hour). When a policy is breached:

1. A `sla.breached` event is emitted to the event bus
2. Space owners receive in-app notifications
3. The late subject is listed under Govern › SLAs and counted on the Govern overview
4. If webhooks are configured, external systems are notified

```bash
# What is late now, most overdue first (viewer; refused for a key limited to some spaces)
GET /api/v1/governance/breaches
```

---

## Govern Overview

The Govern overview (`/govern` in the console) opens on what needs a decision, then draws the figures behind it. Every figure is counted across all spaces, so a key limited to some spaces is refused.

**Needs a decision** — one row per fact, shown only while it is true:

- **Late** — gaps nobody has picked up, assigned gaps with no draft yet, gaps whose draft nobody submitted, and drafts that waited past their review stage, each with how late it is and the term it is judged by.
- **Not set up** (admins only) — no SLA policy while gaps exist, no review workflow while drafts wait, ownership not learned while a source is connected.

**Panels**

- **Ownership coverage** — documents per space, and whether anyone with a role on the space can sign in and has notifications on.
- **SLAs against the clock** — what is late now and by how much.
- **Review flow** (editors and up) — where drafts wait in the default workflow's stages, and what entered, was published or was rejected in the last 30 days.
- **Structure score by space** — scores per space and the weekly line.

**Who sees what**

| | Viewer | Editor and up | Admin |
|---|---|---|---|
| Late rows, coverage, SLAs, structure | yes | yes | yes |
| Names of owners, stewards and assignees; a late row's space | no ("Assigned", counts only) | yes | yes |
| Review flow | no | yes | yes |
| Not-set-up rows | no | no | yes |

A gap's topic is shown only when at least two people other than the reader asked it, or the reader is its assignee or has a role on its space; a late draft's title follows the same rule through the gap it was written for. Who asked is never shown.

How many open gaps each routing step (steward, space owners, team, nobody) would reach is counted only once gap routing runs; until then the overview sends those counts as `null`, never as 0, and shows no "reaches nobody" row.

**API:**
```bash
# The Govern overview: what needs a decision, coverage, SLAs, review flow, structure scores
GET /api/v1/governance/overview
```

The overview needs `viewer`. Capture velocity and top contributors are not part of it.

---

## Configuration

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `SLA_CHECK_INTERVAL_SECS` | `3600` | How often the SLA checker runs (seconds) |
| `SLA_DEFAULT_GAP_ACK_HOURS` | `24` | Default hours before a gap must be picked up (assigned) |
| `SLA_DEFAULT_GAP_RESOLUTION_DAYS` | `7` | Default days before a gap must be resolved |
| `SLA_DEFAULT_DRAFT_REVIEW_HOURS` | `48` | Default hours for draft review completion |
| `SLA_DEFAULT_FRESHNESS_DAYS` | `90` | Default freshness term stored with new policies; no check reads it today |

### config/default.yaml

```yaml
governance:
  sla_check_interval_secs: ${SLA_CHECK_INTERVAL_SECS:-3600}
  default_gap_ack_hours: ${SLA_DEFAULT_GAP_ACK_HOURS:-24}
  default_gap_resolution_days: ${SLA_DEFAULT_GAP_RESOLUTION_DAYS:-7}
  default_draft_review_hours: ${SLA_DEFAULT_DRAFT_REVIEW_HOURS:-48}
  default_freshness_days: ${SLA_DEFAULT_FRESHNESS_DAYS:-90}
```

---

## Roles Required

| Endpoint | Minimum Role |
|---|---|
| Read the Govern overview | `viewer` |
| View coverage report | `viewer` |
| View spaces and stewards | `viewer` |
| Manage space owners | `admin` |
| Manage stewards | `admin` |
| Configure SLA policies | `admin` |
| List what is late now (`/governance/breaches`) | `viewer` |

---

## Workflow: Setting Up Governance for a New Space

1. **Create the space** — Spaces are created implicitly when documents or fragments are assigned to them. Set `space: "PLATFORM"` on ingested docs or captured fragments.

2. **Assign owners** — `POST /api/v1/governance/spaces/PLATFORM/owners` with the team lead's user ID.

3. **Assign stewards** — For specialized topics within the space (e.g., "kubernetes", "ci-cd"), assign subject-matter experts.

4. **Set SLA policies** — Override defaults if the space has stricter requirements (e.g., SRE runbooks need 12h gap acknowledgment).

5. **Monitor** — The Govern overview shows what is late and what needs a decision. SLA breaches trigger notifications automatically.
