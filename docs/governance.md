# Governance

Documentation without ownership decays. DocBrain's governance system makes ownership, accountability, and quality standards explicit — with automated enforcement.

## Overview

Governance in DocBrain has four pillars:

1. **Space Ownership** — Who is responsible for which knowledge areas
2. **Topic Stewardship** — Subject-matter experts for specific topics within spaces
3. **SLA Policies** — Deadlines for gap resolution, draft review, and freshness
4. **Breach Detection** — Automated scanning that surfaces violations before they become incidents

All governance features are accessible from the **Governance** page in the web UI (sidebar → Govern → Governance), which shows the dashboard and rules/owners management via tab navigation.

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
| **Gap Acknowledgment** | Time from gap detection to someone acknowledging it | 24 hours |
| **Gap Resolution** | Time from gap detection to documentation being written | 7 days |
| **Draft Review** | Time from draft submission to review completion | 48 hours |
| **Freshness** | Maximum age before a document is considered stale | 90 days |

### Configuring SLAs

**Web UI:** Governance → Dashboard → SLA section shows compliance. Rules & Owners tab lets you manage per-space policies.

**API:**
```bash
# List all SLA policies
GET /api/v1/governance/slas

# Set default SLA policy
PUT /api/v1/governance/slas/default
{
  "gap_ack_hours": 24,
  "gap_resolution_days": 7,
  "draft_review_hours": 48,
  "freshness_days": 90
}

# Set space-specific SLA (overrides default)
PUT /api/v1/governance/slas/spaces/{space}
{
  "gap_ack_hours": 12,
  "gap_resolution_days": 3,
  "draft_review_hours": 24,
  "freshness_days": 60
}

# Delete a space-specific policy (falls back to default)
DELETE /api/v1/governance/slas/spaces/{space}
```

### Breach Detection

DocBrain runs an automated SLA checker on a configurable interval (default: every hour). When a policy is breached:

1. A `sla.breached` event is emitted to the event bus
2. Space owners receive in-app notifications
3. The breach appears on the governance dashboard
4. If webhooks are configured, external systems are notified

```bash
# List current breaches
GET /api/v1/governance/slas/breaches

# Get breach summary (counts by type and space)
GET /api/v1/governance/slas/breaches/summary

# Acknowledge a breach (stops re-alerting)
POST /api/v1/governance/slas/breaches/{id}/acknowledge
```

---

## Governance Dashboard

The dashboard provides a single view of documentation health:

- **Coverage** — Percentage of topics with documentation per space
- **SLA Compliance** — Which spaces are meeting their SLAs, which are in breach
- **Quality Distribution** — Document scores across spaces (how many are A/B/C/D/F)
- **Capture Velocity** — Fragments captured per week, trending up or down
- **Top Contributors** — Who is contributing the most knowledge, by space

**API:**
```bash
# Full governance dashboard data
GET /api/v1/governance/dashboard

# Coverage report
GET /api/v1/governance/coverage
```

Both require `analyst` role or higher.

---

## Configuration

### Environment Variables

| Variable | Default | Description |
|---|---|---|
| `SLA_CHECK_INTERVAL_SECS` | `3600` | How often the SLA checker runs (seconds) |
| `SLA_DEFAULT_GAP_ACK_HOURS` | `24` | Default hours before a gap must be acknowledged |
| `SLA_DEFAULT_GAP_RESOLUTION_DAYS` | `7` | Default days before a gap must be resolved |
| `SLA_DEFAULT_DRAFT_REVIEW_HOURS` | `48` | Default hours for draft review completion |
| `SLA_DEFAULT_FRESHNESS_DAYS` | `90` | Default days before a document is stale |

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
| View governance dashboard | `analyst` |
| View coverage report | `viewer` |
| View spaces and stewards | `viewer` |
| Manage space owners | `admin` |
| Manage stewards | `admin` |
| Configure SLA policies | `admin` |
| View/acknowledge breaches | `analyst` |

---

## Workflow: Setting Up Governance for a New Space

1. **Create the space** — Spaces are created implicitly when documents or fragments are assigned to them. Set `space: "PLATFORM"` on ingested docs or captured fragments.

2. **Assign owners** — `POST /api/v1/governance/spaces/PLATFORM/owners` with the team lead's user ID.

3. **Assign stewards** — For specialized topics within the space (e.g., "kubernetes", "ci-cd"), assign subject-matter experts.

4. **Set SLA policies** — Override defaults if the space has stricter requirements (e.g., SRE runbooks need 12h gap acknowledgment).

5. **Monitor** — The governance dashboard shows compliance. SLA breaches trigger notifications automatically.
