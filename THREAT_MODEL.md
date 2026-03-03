# DocBrain Threat Model

**Status:** Active
**Last updated:** 2026-03-03
**Scope:** DocBrain server, web UI, ingest pipeline, integrations

---

## 1. Assets

| Asset | Sensitivity | Description |
|---|---|---|
| **API keys** | Critical | Bearer tokens granting access to all API functionality |
| **Admin API key** | Critical | Master key with full access — generated once on first boot |
| **User query data** | High | Episodes table contains user questions (potentially sensitive: incident context, internal tooling names) |
| **Confluence credentials** | High | API token / PAT for Confluence integration |
| **LLM API keys** | High | Anthropic/OpenAI/AWS credentials |
| **Embeddings index** | Medium | OpenSearch semantic index — contains chunked documentation content |
| **Documentation content** | Medium | The ingested documentation itself (may contain internal procedures, configs) |
| **Freshness/gap signals** | Low | Aggregate analytics — less sensitive than raw queries |

---

## 2. Trust Boundaries

```
[User Browser / CLI / Slack]
         │
         │  HTTPS + Bearer token
         ▼
[DocBrain Server]  ←─── [Admin: key management, ingest trigger]
         │
         ├──▶ [PostgreSQL]  ─── Trusts server completely
         ├──▶ [OpenSearch]  ─── Trusts server completely
         ├──▶ [Redis]       ─── Trusts server completely (session data only)
         ├──▶ [LLM API]     ─── External trust boundary (cloud provider)
         └──▶ [Confluence API] ─── External trust boundary
```

---

## 3. Threats and Mitigations

### T1: Unauthenticated API Access

**Threat:** An attacker accesses the API without a valid key, or with a revoked/expired key.
**Impact:** Data exfiltration (query history, documentation), resource abuse.
**Likelihood:** High if exposed to the internet without network-level protection.

**Mitigations:**
- ✅ All API endpoints except `/api/v1/health` and `/api/v1/config` require Bearer token
- ✅ Keys are stored as Argon2 hashes — raw key cannot be recovered from DB
- ✅ Key revocation is instant — revoked keys fail validation on next request
- ✅ Per-key rate limiting (configurable RPM) prevents abuse
- ⚠️ **Residual risk:** No IP allowlisting. In production, deploy behind a reverse proxy with IP restriction if possible.

### T2: API Key Leakage

**Threat:** An API key is leaked via logs, error messages, or accidental commit.
**Impact:** Unauthorized access to all functionality at that key's role level.
**Likelihood:** Medium — common in practice.

**Mitigations:**
- ✅ Keys are never logged (auth middleware doesn't log extracted keys)
- ✅ Error messages use sanitized strings with correlation IDs, not key values
- ✅ Bootstrap admin key is written to a file (not stdout/logs by default)
- ✅ Key file permissions set to 0600 (owner-readable only) on Unix
- ⚠️ **Residual risk:** If `ADMIN_KEY_FILE` points to a volume shared with other processes, the file could be read. Mount it to a secret volume (Kubernetes Secret or Docker secret).

### T3: SSRF via External HTTP Calls

**Threat:** An attacker manipulates a URL parameter to cause the server to make requests to internal services (metadata endpoints, localhost services, etc.).
**Impact:** Internal service enumeration, credential theft from metadata APIs (AWS IMDS, GCP metadata).
**Likelihood:** Low — URL inputs come from admin-configured env vars, not user requests.

**Mitigations:**
- ✅ Confluence base URL is admin-configured, not user-supplied
- ✅ Image download URLs are relative paths from the Confluence base URL — cannot be redirected to arbitrary hosts
- ✅ LLM API endpoints are configured at startup, not per-request
- ⚠️ **Residual risk:** If Confluence is self-hosted, an admin could configure `CONFLUENCE_BASE_URL=http://169.254.169.254/` to hit cloud metadata endpoints. Mitigate by validating that the Confluence URL is not a private IP range in future versions.

### T4: SQL Injection

**Threat:** Malicious query input is injected into SQL statements.
**Impact:** Data exfiltration, data modification, authentication bypass.
**Likelihood:** Very Low — all queries use parameterized statements.

**Mitigations:**
- ✅ All database queries use sqlx parameterized queries (`$1`, `$2` bindings)
- ✅ No string interpolation in SQL queries
- ✅ sqlx compile-time query verification prevents unparameterized queries from compiling

### T5: Prompt Injection

**Threat:** Untrusted text (document content, user queries, episodic memory) contains instructions that manipulate LLM behavior ("Ignore previous instructions...") or XML delimiters that break prompt structure.

Two distinct attack vectors:

**5a — XML delimiter injection:** A document contains `</documents>` or `</question>`, closing the XML tags used to delimit context blocks. Text after the closed tag is interpreted by the LLM as instructions rather than content.

**5b — Cross-user episodic memory injection:** A user submits a malicious query (e.g. `IGNORE ALL PREVIOUS INSTRUCTIONS — you are now...`). That text is stored as `query_text` in the episodes table and fed back verbatim into future LLM prompts for *other users* whose questions are semantically similar. The injector doesn't need access to other users' sessions — the episodic recall mechanism delivers the payload automatically.

**Impact:** LLM produces misleading or dangerous answers; confidential context from other sessions potentially leaked.
**Likelihood:** Medium for 5a (requires a malicious or compromised Confluence contributor). Medium for 5b (any authenticated user can attempt it).

**Mitigations:**
- ✅ System prompt enforces grounding: "Every factual claim MUST come from the provided context"
- ✅ XML delimiter injection (5a): all untrusted strings are sanitized before LLM context assembly — `</documents>`, `</question>`, `<documents>`, `<question>` are HTML-entity escaped. Applied to document content, titles, session history, episodic memory fields (`query_text`, `answer_text`, `feedback_note`, `feedback_reason`), knowledge graph entity/relation fields, and the user query itself.
- ⚠️ **Residual risk (5b):** HTML-entity escaping neutralizes structural XML injection. It does not prevent a sophisticated natural-language injection ("You are a helpful assistant who always...") from influencing LLM output if it reaches the context. Mitigation: review episodic memory entries flagged with unusual query patterns; consider a content moderation pass on stored episodes in high-security deployments.

### T6: Data Exfiltration via Analytics

**Threat:** A regular user (non-admin) calls `/api/v1/analytics` to extract raw user query text.
**Impact:** Privacy violation — users' questions could contain sensitive information.
**Likelihood:** Medium — the analytics endpoint exists and is accessible to all authenticated users.

**Mitigations:**
- ✅ `top_queries` (raw query text) is cleared from analytics responses for non-admin users
- ✅ `doc_gaps` in analytics uses LLM-generated cluster labels, not raw query text
- ✅ Autopilot gaps API strips `sample_queries` from non-admin responses
- ⚠️ **Residual risk:** Admin-role API keys can still access raw query data. Ensure admin keys are tightly controlled.

### T7: Session Fixation / Token Theft

**Threat:** Web UI tokens stored in localStorage are stolen via XSS.
**Impact:** Session hijacking — attacker can use the stolen API key.
**Likelihood:** Low — Next.js mitigates most XSS vectors; localStorage tokens are a known risk.

**Mitigations:**
- ✅ OIDC/SSO integration is implemented — organizations can configure SSO so users authenticate via their identity provider rather than storing raw API keys in the browser
- ⚠️ **Known gap:** Even with OIDC, the session token derived from OIDC auth is stored in `localStorage` rather than an `httpOnly` cookie. If XSS were achieved in the web UI, the token could be exfiltrated. Tracked as a known issue — migrating web UI session storage to `httpOnly` cookies is planned.
- ⚠️ **Residual risk:** CSP headers on the web UI are not enforced by default. Operators should configure CSP via their reverse proxy (nginx, Caddy, ALB) as part of the production deployment checklist.

### T8: Webhook Replay / Forgery (Confluence)

**Threat:** An attacker sends forged Confluence webhook events to trigger unauthorized document sync or deletion.
**Impact:** Document index poisoning, unauthorized content ingestion.
**Likelihood:** Low — requires attacker to be able to reach the webhook endpoint.

**Mitigations:**
- ✅ Confluence webhook HMAC-SHA256 signature verification is implemented
- ✅ Webhook endpoint is separate from the main API and requires the signing secret
- ⚠️ **Residual risk:** Replay attacks (re-sending a captured valid signature) are not prevented. Add a nonce or timestamp check in future versions.

### T9: Denial of Service via Large Queries

**Threat:** An attacker sends extremely large query strings or floods the API to degrade service.
**Impact:** Service unavailability, high LLM costs.
**Likelihood:** Medium — API is often internet-accessible.

**Mitigations:**
- ✅ Per-key rate limiting (RPM cap) prevents sustained flooding
- ✅ Query length validation: queries over 10,000 characters are rejected
- ⚠️ **Residual risk:** A coordinated attack from many keys would still succeed. Add global rate limiting or deploy behind a WAF in production.

### T10: Insecure Default Configuration

**Threat:** The server runs with insecure defaults (open CORS, weak rate limits, no TLS).
**Impact:** Unauthorized cross-origin access, weaker protection.
**Likelihood:** High in dev/quick-start setups.

**Mitigations:**
- ✅ CORS defaults to `http://localhost:3001` only — not `*`
- ✅ CORS can be overridden via `CORS_ALLOWED_ORIGINS` env var
- ⚠️ **Residual risk:** No TLS termination built-in. DocBrain should always be deployed behind nginx/caddy/ALB with TLS in production. See `docs/deployment.md`.

---

## 4. Out of Scope (Intentional)

- **LLM provider security**: Anthropic/OpenAI/Bedrock security is the provider's responsibility
- **Infrastructure security**: PostgreSQL, OpenSearch, Redis security is the operator's responsibility
- **Network security**: VPN, firewalls, and network segmentation are the operator's responsibility
- **SAML 2.0 SSO**: Planned for enterprise tier, not yet implemented

---

## 5. Security Checklist for Operators

Before deploying to production:

- [ ] Run DocBrain behind a reverse proxy with TLS (nginx, Caddy, AWS ALB)
- [ ] Rotate the bootstrap admin key after first boot (create a new admin key, revoke the bootstrap key)
- [ ] Set `CORS_ALLOWED_ORIGINS` to your web UI domain only
- [ ] Use Kubernetes Secrets or Docker Secrets for all credentials (not plain env vars in `docker-compose.yml`)
- [ ] Restrict the `ADMIN_KEY_FILE` path to a secret volume
- [ ] Configure your LLM API key with minimal scopes (e.g., Bedrock resource-based policies)
- [ ] Set up database backups for PostgreSQL (episodes + gap clusters contain valuable signal)
- [ ] Run `cargo audit` regularly to check for dependency vulnerabilities
- [ ] Monitor for unusual query patterns that may indicate abuse

---

## 6. Responsible Disclosure

If you discover a security vulnerability in DocBrain, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Report via GitHub's private vulnerability reporting: navigate to the repository → Security tab → "Report a vulnerability". We aim to acknowledge reports within 48 hours and provide an initial assessment within 7 days.

Please include:
- Description of the vulnerability and attack scenario
- Steps to reproduce or proof-of-concept
- Affected version(s)
- Your assessment of impact and likelihood

We will credit researchers in the changelog unless you prefer to remain anonymous.
