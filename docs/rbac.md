# DocBrain RBAC & SSO Access Control

DocBrain supports three SSO providers for login — **GitHub OAuth**, **GitLab OIDC**, and **generic OIDC** (Azure AD, Okta, Google Workspace, Keycloak, etc.) — each with a consistent RBAC model.

---

## Roles

| Role | What they can do |
|------|-----------------|
| `viewer` | Ask questions, view answers, give feedback, and access all intelligence dashboards (Documentation Analytics, Predictive Gaps, Autonomous Document Maintenance, Knowledge Stream). Default for new SSO users. |
| `editor` | Everything viewer can + manage knowledge spaces and captures. |
| `analyst` | Everything editor can. Reserved for future role-based scoping; currently equivalent to `editor`. |
| `admin` | Full access: manage users, API keys, RBAC config, trigger ingests. |

---

## Configuring SSO Providers

### GitHub OAuth

1. Create an OAuth App at **GitHub → Settings → Developer settings → OAuth Apps → New OAuth App**
2. Set **Callback URL**: `https://<your-domain>/api/v1/auth/github/callback`
3. Note the **Client ID** and generate a **Client Secret**

**Helm values:**
```yaml
githubOAuth:
  enabled: true
  clientId: "Iv1.abc123"
  clientSecret: "your-secret"   # stored in the Kubernetes Secret
  redirectUri: "https://docbrain.acme.com/api/v1/auth/github/callback"
```

**CLI login:**
```bash
docbrain login --github
# Opens browser → authorize → session saved automatically
```

**Group source:** GitHub org memberships (fetched via `/user/orgs`). The `read:org` scope is required if you want group-based access control. To add it, go to your OAuth App settings and add `read:org` to the requested scopes, or set `GITHUB_OAUTH_SCOPES=user:email,read:org` in `server.env`.

---

### GitLab OIDC

1. Go to **GitLab → Admin Area → Applications → New application** (or user/group-level)
2. Enable scopes: `openid`, `profile`, `email`, `groups`
3. Set **Redirect URI**: `https://<your-domain>/api/v1/auth/gitlab/callback`

**Helm values:**
```yaml
gitlabOAuth:
  enabled: true
  instanceUrl: "https://gitlab.com"   # or your self-hosted URL
  clientId: "your-app-id"
  clientSecret: "your-secret"
  redirectUri: "https://docbrain.acme.com/api/v1/auth/gitlab/callback"
```

**CLI login:**
```bash
docbrain login --gitlab
```

**Group source:** GitLab includes a `groups` array in the ID token when the `groups` scope is requested. This contains the full paths of groups the user belongs to (e.g. `acme/platform-team`).

---

### Generic OIDC (Azure AD, Okta, Google, Keycloak)

**Helm values:**
```yaml
oidc:
  enabled: true
  issuerUrl: "https://login.microsoftonline.com/<tenant>/v2.0"
  clientId: "your-client-id"
  clientSecret: "your-secret"
  redirectUri: "https://docbrain.acme.com/api/v1/auth/oidc/callback"
  webUiUrl: "https://docbrain.acme.com"
```

**CLI login:**
```bash
docbrain login --oidc
```

**Group source:** The `groups` array claim in the ID token. Each provider surfaces this differently:

| Provider | How to enable groups claim |
|----------|---------------------------|
| Azure AD | Add "groups" optional claim in App Registration → Token configuration |
| Okta | Add Groups claim to the Authorization Server policy |
| Google Workspace | Use the `https://www.googleapis.com/auth/cloud-identity.groups.readonly` scope (requires Workspace admin) |
| Keycloak | Add "groups" to the client's mapper config |

---

## Role Assignment

Role is computed at login time in this priority order (highest wins):

| Priority | Check | Config | Example |
|----------|-------|--------|---------|
| 1 | Email in admin list | `rbac.adminEmails` | `alice@acme.com,bob@acme.com` |
| 2 | Email domain → admin | `rbac.adminDomain` | `acme.com` |
| 3 | Member of admin group | `rbac.adminGroups` | `platform-team,sre-team` |
| 4 | Member of editor group | `rbac.editorGroups` | `docs-team` |
| 5 | Default | `rbac.defaultRole` | `viewer` (default) |

**Helm example — auto-promote platform team to admin:**
```yaml
rbac:
  defaultRole: "viewer"
  adminGroups: "platform-team,sre-team"
  editorGroups: "docs-writers"
```

---

## Access Gate (Allowlist)

By default, **any user who can authenticate** with the configured provider is allowed in. To restrict access to specific groups or domains only:

### Allow specific groups only

```yaml
rbac:
  allowedGroups: "platform-team,sre-team,docs-team"
```

Anyone not in one of those groups gets `403 Forbidden` at the OAuth callback — **the user is never created in the database**.

### Allow specific email domains only

```yaml
rbac:
  allowedDomains: "acme.com,acme.org"
```

Users with `@acme.com` or `@acme.org` email addresses are allowed; all others are denied.

### Both together (OR logic)

```yaml
rbac:
  allowedGroups: "platform-team"
  allowedDomains: "acme.com"
```

A user passes if they satisfy **either** restriction — member of `platform-team` *or* has an `@acme.com` email. This lets you allow contractors (by domain) while also letting external collaborators in if they're explicitly added to the group.

### Combining access gate with role assignment

```yaml
rbac:
  allowedDomains: "acme.com"        # only @acme.com users may log in
  adminGroups: "platform-team"       # @acme.com members of platform-team get admin
  defaultRole: "viewer"              # everyone else gets viewer
```

---

## Access Control for CLI, Slack, and Webhook Captures

The access gate applies uniformly:

| Entry point | How the gate is applied |
|-------------|------------------------|
| `docbrain login --github/--gitlab/--oidc` | Checked at OAuth callback; CLI receives 403 if denied |
| Web UI login | Same callback; browser redirected to error page |
| Slack `/docbrain` command | Slack user must have a linked DocBrain account (created via login); their role governs what they can do |
| GitHub/GitLab webhook captures | These use a **bot API key** (admin-scoped), not user OAuth — the gate does not apply to webhook-initiated ingestion |
| MCP server | Uses a PAT (`DOCBRAIN_API_KEY`) created by an authorised user via `docbrain token create` |

---

## Environment Variables Reference

All values can also be set directly as environment variables (useful for docker-compose or non-Helm deployments).

| Env var | Helm value | Description |
|---------|-----------|-------------|
| `GITHUB_CLIENT_ID` | `githubOAuth.clientId` | GitHub OAuth App client ID |
| `GITHUB_CLIENT_SECRET` | `githubOAuth.clientSecret` | GitHub OAuth App client secret |
| `GITHUB_REDIRECT_URI` | `githubOAuth.redirectUri` | Must match registered callback URL |
| `GITLAB_INSTANCE_URL` | `gitlabOAuth.instanceUrl` | GitLab base URL (default: `https://gitlab.com`) |
| `GITLAB_CLIENT_ID` | `gitlabOAuth.clientId` | GitLab application ID |
| `GITLAB_CLIENT_SECRET` | `gitlabOAuth.clientSecret` | GitLab application secret |
| `GITLAB_REDIRECT_URI` | `gitlabOAuth.redirectUri` | Must match registered callback URL |
| `OIDC_ISSUER_URL` | `oidc.issuerUrl` | OIDC discovery URL (without `/.well-known/openid-configuration`) |
| `OIDC_CLIENT_ID` | `oidc.clientId` | OIDC client/application ID |
| `OIDC_CLIENT_SECRET` | `oidc.clientSecret` | OIDC client secret |
| `OIDC_REDIRECT_URI` | `oidc.redirectUri` | Must match registered callback URL |
| `OIDC_WEB_UI_URL` | `oidc.webUiUrl` | Web UI base URL for post-login redirect |
| `OIDC_DEFAULT_ROLE` | `rbac.defaultRole` | Default role for new SSO users (`viewer`) |
| `OIDC_ADMIN_DOMAIN` | `rbac.adminDomain` | Email domain → admin role |
| `OIDC_ADMIN_EMAILS` | `rbac.adminEmails` | Specific emails → admin role |
| `OIDC_ADMIN_GROUPS` | `rbac.adminGroups` | IdP groups → admin role |
| `OIDC_EDITOR_GROUPS` | `rbac.editorGroups` | IdP groups → editor role |
| `OIDC_ALLOWED_GROUPS` | `rbac.allowedGroups` | **Access gate**: groups allowed to log in |
| `OIDC_ALLOWED_DOMAINS` | `rbac.allowedDomains` | **Access gate**: email domains allowed to log in |

---

## Troubleshooting

**User gets 403 at login**
- Check `OIDC_ALLOWED_GROUPS` and `OIDC_ALLOWED_DOMAINS` — the user doesn't satisfy either restriction.
- For GitHub: verify the user is a *public* member of the org, or that the OAuth App has `read:org` scope (private membership requires this).
- For GitLab: verify the `groups` scope is enabled on the application and the user is a direct member (not just inherited).
- For OIDC: check your provider's admin console to confirm the `groups` claim is populated in the token.

**Groups claim is empty / role assignment not working**
- Use a JWT decoder (e.g. [jwt.io](https://jwt.io)) to inspect the ID token and confirm `groups` is present.
- Azure AD: ensure you added the Groups claim and the user is assigned to the app.
- Okta: the Groups claim must be added to both the Access Token and ID Token policies.

**Existing users not re-evaluated on group change**
- Role is computed at login time and stored on the user record. If you change `adminGroups`, users must log out and log back in for the new role to take effect.
