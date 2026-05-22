# Access Control (ACL)

DocBrain enforces source-system access controls at query time. When a
user asks a question, results are filtered to only the content that
user is authorised to read in the source system (Confluence,
Slack, GitHub, Jira) — regardless of how DocBrain itself is configured.

## Why this matters

Without ACL, a RAG system flattens every source's permission model
into one shared index. A Confluence page restricted to your Finance
team becomes readable by every DocBrain user. A private Slack channel
discussion surfaces in a marketing intern's question. Sensitive
GitHub repos leak through search results.

DocBrain solves this with **source-system ACL mirroring**: every
chunk in the index carries the principals (users, groups, channels,
roles) authorised to read it at the source. Every retrieval filters
by the requesting user's resolved principal set.

## Quick start

1. **Configure your IdP** so SSO group claims reach DocBrain (see
   [RBAC & SSO](rbac.md)).
2. **Turn on ACL mirroring** for the sources you care about. Default
   is off — fully backwards compatible.

   ```yaml
   acl:
     mode: enforce              # off | warn | enforce
     sources:
       confluence:
         mode: mirror           # extract real Confluence restrictions
       github:
         mode: mirror           # extract repo visibility + collaborators
       slack:
         mode: mirror           # extract channel membership
       jira:
         mode: mirror           # extract issue security level
   ```

3. **Backfill existing chunks** (one-time, for content that pre-dates
   the ACL mirroring deploy):

   ```bash
   kubectl create job --from=cronjob/docbrain-ingest acl-backfill \
     -n docbrain --overrides '{"spec":{"template":{"spec":{"containers":[{"name":"ingest","command":["docbrain-acl-backfill"],"env":[{"name":"ACL_BACKFILL_SOURCE","value":"confluence"}]}]}}}}'
   ```

4. **Optional: validate before enforcing.** Set `mode: warn` first.
   The system computes filters and logs `would-have-denied` chunks
   without actually changing user-visible results. Operators run
   this for a few days to validate ACL coverage before flipping to
   `enforce`.

## How it works

### At ingest

Per-source `AclProvider` impls extract real permission data:

| Source | What it captures |
|---|---|
| **Confluence** | Page-level read restrictions; falls back to space permission scheme |
| **GitHub** | Repo visibility (public / internal / private) + collaborators + teams |
| **Slack** | Channel membership for private channels; workspace group for public |
| **Jira** | Issue security level + project-level Browse permission |

Each provider returns a `DocumentAcl` with the principal set. The
ingest pipeline persists it to Postgres (`document_acl` table) and
writes the canonical principal strings into the OpenSearch chunk
index (`acl_principals` keyword array).

### At query

When a user asks a question:

1. **User identity → principal set.** Their DocBrain user, email,
   linked external identities (GitHub login, Slack user ID), and
   SSO group memberships all become principals.
2. **Filter every retriever.** Each retriever's hits are intersected
   with the user's principals. Chunks the user can't read are
   dropped before they reach the reranker, the LLM, or the response.
3. **Side-channel mitigations.** When the filter wipes out the
   result set, the side-channel guard fires:
   - Answer text is replaced with the configured denial message
   - Confidence is forced to 0.0
   - Sources are emptied
   - Episodic memory persistence is skipped (no future leak via
     past-question lookup)

### Modes

```yaml
acl:
  mode: off | warn | enforce
```

| Mode | Behaviour |
|---|---|
| **off** (default) | ACL filter not applied. Backwards-compatible. |
| **warn** | Filter computed but not applied. Logs `would-have-denied` chunk_ids on every query. Use to validate coverage before enforcing. |
| **enforce** | Filter applied. Denied chunks dropped. Fail-closed on chunks lacking ACL data (configurable). |

## Per-source policy

Operators control how each source's ACL is treated independently:

```yaml
acl:
  sources:
    confluence:
      mode: mirror
      space_overrides:
        ENG: public                 # treat Engineering as broadly shareable
        HR: admin_only              # restrict HR to admins only
    slack:
      mode: mirror
    github:
      mode: mirror
    jira:
      mode: mirror
```

| Per-source mode | Meaning |
|---|---|
| `off` | Don't extract ACL. Chunks have no ACL data; query filter treats per `unknown_policy`. |
| `mirror` | Extract real per-document ACL from the source (the default for production deployments). |
| `public` | Tag with the Public sentinel — any authenticated DocBrain user can read. |
| `admin_only` | Tag with the admin role principal — only admins can read. |

## Denial UX (RFC-002)

When the filter denies content, the system communicates the denial in
one of three modes — operator-configurable per deployment:

```yaml
acl:
  denial:
    mode: silent | disclosed_no_count | disclosed
    referral: "your administrator"
```

| Mode | What the user sees | When to use |
|---|---|---|
| **silent** | Generic *"I don't have information"*; no metadata leak | MNPI / SEC-regulated content; existence of restricted material is itself non-public |
| **disclosed_no_count** *(default)* | *"This answer reflects only content you have access to. Some related material may exist but is restricted by your team's permissions. Contact your administrator..."* | Standard enterprise — users know the system is filtering, no specifics leak |
| **disclosed** | Includes denied count + referral string verbatim | Open-collaboration orgs prioritising fast self-service unblocking |

### Per-source override (strictest wins)

Different sources can have different denial policies. When a query
touches multiple sources with different settings, **the strictest
mode wins** to prevent leak by inclusion:

```yaml
acl:
  denial:
    mode: disclosed_no_count
    source_overrides:
      confluence:
        space_overrides:
          FINANCE: { mode: silent }   # Finance space → always silent
          MNPI: { mode: silent }
      slack:
        channel_overrides:
          incidents-finance: { mode: silent }
```

A query that returns chunks from both `ENG` (disclosed_no_count) and
`FINANCE` (silent) → the response uses `silent` mode. Mixing public
and MNPI content never weakens the response.

### Per-role override

Admins always get full disclosure by default so they can debug:

```yaml
acl:
  denial:
    role_overrides:
      admin: disclosed
      analyst: disclosed_no_count    # explicit override (default: inherit)
```

## Audit log

For HIPAA / FedRAMP / SOC2 deployments, every full or partial denial
can be persisted to a durable audit log:

```yaml
acl:
  denial:
    audit: true
    audit_raw_query: false           # default: SHA256-hash the query
```

Each row records:
- `decided_at` — when the denial fired
- `user_id` — the DocBrain user (NULL on user delete via CASCADE SET NULL)
- `query_hash` — SHA256 of the query (or raw text if `audit_raw_query: true`)
- `decision` — `full_deny` or `partial_deny`
- `denial_mode` — the resolved mode the response used
- `denied_breakdown` — `[{source_type, namespace, count}, ...]`
- `denied_count` — total across all sources
- `policy_chain` — provenance for the resolved decision

Queries are SHA256-hashed by default — queries can themselves be
MNPI ("what's the Q3 earnings number?"). Operators flip
`audit_raw_query: true` only with explicit compliance approval.

## Response shape

Every `/api/v1/ask` response includes a structured `access` field
when the filter fires:

```json
{
  "answer": "...",
  "sources": [...],
  "confidence": 0.85,
  "access": {
    "mode": "disclosed_no_count",
    "filter_applied": true,
    "fully_denied": false,
    "denied_count": 0,
    "referral": "your administrator"
  }
}
```

| Field | Meaning |
|---|---|
| `mode` | Resolved denial mode for this response |
| `filter_applied` | True iff at least one chunk was denied |
| `fully_denied` | True iff the filter emptied the result set entirely |
| `denied_count` | Number of denied chunks. Zeroed in `silent` and `disclosed_no_count` modes. |
| `referral` | Operator-configured referral string |

The `access` field is absent when the filter doesn't fire (no
denials happened) — keeps responses small for the common case.

## Diagnostics

### "What can I see?"

Authenticated users can hit `GET /api/v1/me/acl` to see their
resolved principal set:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  https://docbrain.example.com/api/v1/me/acl
```

```json
{
  "user_id": "...",
  "email": "...",
  "principals": [
    {"canonical": "public:system:public", "kind": "public", "origin": "synthetic"},
    {"canonical": "user:sso:alice@example.com", "kind": "user", "origin": "email"},
    {"canonical": "sso_group:sso:engineering", "kind": "sso_group", "origin": "oidc_claim"},
    {"canonical": "user:slack:U123ABC", "kind": "user", "origin": "external_identity:slack"}
  ]
}
```

Useful for users to debug "why didn't I get this answer?"

### Coverage report

Admins can check what fraction of the corpus has ACL data via
SQL on the `document_acl` table:

```sql
SELECT
  (SELECT COUNT(*) FROM documents WHERE deleted_at IS NULL) AS total,
  (SELECT COUNT(DISTINCT document_id) FROM document_acl da
   JOIN documents d ON d.id = da.document_id WHERE d.deleted_at IS NULL) AS with_acl;
```

## Threat model summary

| Threat | Mitigation |
|---|---|
| Cross-tenant chunk leak | Filter applied at every retriever output; defense-in-depth post-filter on by-id retrievers |
| Stale ACL leakage | `max_age_at_query` hard fence + periodic refresh + webhook subscriptions |
| Existence side-channel | Confidence forced to 0 on empty filtered result; episode/cache keyed on principal hash |
| Cross-user cache poisoning | Cache key includes principal set hash |
| Episode/feedback leakage | Persistence skipped when filter fully denies |
| Bootstrap-admin bypass | Configurable, defaults to `[Public, World]` only — bootstrap keys do NOT bypass ACL |
| Connector ACL bug | `acl_capable() = false` ⇒ admin-only ingest fallback |
| Mixing MNPI + public in one response | "Strictest wins" denial-mode resolver |

## Reference

- **Trait:** `AclProvider` in `docbrain-ingest/src/acl_provider.rs`
- **Postgres tables:** `principals`, `document_acl`, `user_principals`,
  `user_external_identities`, `acl_audit_log`
- **OpenSearch field:** `acl_principals` (keyword array on each chunk)
- **Diagnostics endpoint:** `GET /api/v1/me/acl`
- **Backfill binary:** `docbrain-acl-backfill`

## Configuration reference

See [Configuration](configuration.md#acl) for the full env-var reference.
