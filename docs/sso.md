# Single Sign-On (SSO) Configuration

DocBrain supports three SSO providers. All three can be active simultaneously — the login screen shows only the buttons that are actually configured.

| Provider | Protocol | Discovery | ID Token |
|---|---|---|---|
| Generic OIDC | OpenID Connect | Yes (`/.well-known/openid-configuration`) | Yes |
| GitLab | OpenID Connect | Yes | Yes |
| GitHub | OAuth 2.0 | No | No |

---

## Generic OIDC

Works with any OpenID Connect compliant identity provider: Google Workspace, Okta, Auth0, Azure AD, Keycloak, Dex, and others.

### Setup

1. Register a new OAuth application / OIDC client in your IdP.
2. Set the redirect URI to `https://your-docbrain-host/api/v1/auth/oidc/callback`.
3. Request scopes: `openid`, `email`, `profile`.
4. Set env vars:

```env
OIDC_ISSUER_URL=https://accounts.google.com   # or your IdP's issuer URL
OIDC_CLIENT_ID=your-client-id
OIDC_CLIENT_SECRET=your-client-secret
OIDC_REDIRECT_URI=https://your-docbrain-host/api/v1/auth/oidc/callback
OIDC_WEB_UI_URL=https://your-web-ui-host       # where to redirect after login (default: http://localhost:3001)
```

DocBrain discovers the provider's endpoints automatically via the standard OIDC discovery URL. No manual endpoint configuration needed.

### Flow

```
Browser → GET /api/v1/auth/oidc/start
        ← 302 redirect to IdP (with PKCE + state)
IdP     → GET /api/v1/auth/oidc/callback?code=…&state=…
        → validates signature, extracts email+name+sub
        → upserts user in DB with computed role
        → generates a single-use OTP (60-second TTL)
        ← 302 redirect to OIDC_WEB_UI_URL?code=otp_<uuid>
Frontend → POST /api/v1/auth/exchange { "code": "otp_<uuid>" }
         ← { "api_key": "db_sk_…", … }   (token in response body, never in URL)
         → calls history.replaceState to clear the code from the URL
```

PKCE (S256) is used on all flows. The code verifier and CSRF state are stored in Redis with a 5-minute TTL. The OTP is single-use and stored in Redis with a 60-second TTL — it cannot be replayed and does not appear in server logs or browser history after the exchange.

---

## GitHub OAuth 2.0

GitHub does not support OIDC (no ID token, no discovery endpoint). DocBrain implements the standard OAuth 2.0 Authorization Code flow directly.

### Setup

1. Go to **GitHub → Settings → Developer settings → OAuth Apps → New OAuth App**.
2. Set **Authorization callback URL** to `https://your-docbrain-host/api/v1/auth/github/callback`.
3. Copy the Client ID and generate a Client Secret.
4. Set env vars:

```env
GITHUB_CLIENT_ID=your-github-client-id
GITHUB_CLIENT_SECRET=your-github-client-secret
GITHUB_REDIRECT_URI=https://your-docbrain-host/api/v1/auth/github/callback
```

### How email is resolved

GitHub allows users to hide their primary email. DocBrain resolves email in this order:
1. `user.email` from `GET /user` (if non-empty)
2. Primary verified email from `GET /user/emails`

If no verified email is found, login is rejected with a clear error.

### Flow

```
Browser → GET /api/v1/auth/github/start
        ← 302 redirect to github.com/login/oauth/authorize
GitHub  → GET /api/v1/auth/github/callback?code=…&state=…
        → exchanges code for access token
        → fetches user profile + email
        → upserts user with computed role
        → generates a single-use OTP (60-second TTL)
        ← 302 redirect to OIDC_WEB_UI_URL?code=otp_<uuid>
Frontend → POST /api/v1/auth/exchange { "code": "otp_<uuid>" }
         ← { "api_key": "db_sk_…", … }   (token in response body, never in URL)
         → calls history.replaceState to clear the code from the URL
```

---

## GitLab OIDC

GitLab supports OIDC natively. Both GitLab.com (SaaS) and self-managed instances work.

### Setup

1. Go to **GitLab → Applications** (group or user level, or admin-level for instance-wide).
2. Set **Redirect URI** to `https://your-docbrain-host/api/v1/auth/gitlab/callback`.
3. Enable scopes: `openid`, `email`, `profile`.
4. Set env vars:

```env
GITLAB_OIDC_ISSUER_URL=https://gitlab.com     # or your self-managed instance URL
GITLAB_CLIENT_ID=your-gitlab-client-id
GITLAB_CLIENT_SECRET=your-gitlab-client-secret
GITLAB_REDIRECT_URI=https://your-docbrain-host/api/v1/auth/gitlab/callback
```

GitLab's OIDC discovery URL is `{GITLAB_OIDC_ISSUER_URL}/.well-known/openid-configuration`.

---

## Microsoft / Azure AD (Entra ID)

Azure AD (now branded Entra ID) works with the generic OIDC provider.

### Setup

1. Sign in to [portal.azure.com](https://portal.azure.com) and search for **Entra ID** (formerly Azure Active Directory).
2. Go to **App registrations** → **New registration**.
   - Name: e.g. "DocBrain"
   - Supported account types: choose the appropriate scope for your org
   - Redirect URI (Web): `https://your-docbrain-host/api/v1/auth/oidc/callback`
3. After registration, note the **Application (client) ID** and **Directory (tenant) ID**.
4. Go to **Certificates & secrets** → **New client secret**. Copy the **Value** (shown only once).
5. Go to **API permissions** → **Add a permission** → **Microsoft Graph** → **Delegated permissions**:
   - Add: `openid`, `profile`, `email`
   - Click **Grant admin consent** for your directory.

```env
OIDC_ISSUER_URL=https://login.microsoftonline.com/<TENANT_ID>/v2.0
OIDC_CLIENT_ID=<APPLICATION_CLIENT_ID>
OIDC_CLIENT_SECRET=<CLIENT_SECRET_VALUE>
OIDC_REDIRECT_URI=https://your-docbrain-host/api/v1/auth/oidc/callback
OIDC_WEB_UI_URL=https://your-web-ui-host
```

DocBrain discovers all endpoints automatically via `https://login.microsoftonline.com/<TENANT_ID>/v2.0/.well-known/openid-configuration`. No manual endpoint configuration needed.

---

## ADFS (Active Directory Federation Services)

ADFS works as an OIDC provider via the generic OIDC flow.

### Setup

1. Open **AD FS Management** on your ADFS server.
2. Go to **Application Groups** → **Add Application Group**.
   - Template: **Server application accessing a web API** (or "Native application + Web API" depending on your ADFS version)
3. **Server application** step:
   - Note the generated **Client Identifier** — this is your `OIDC_CLIENT_ID`.
   - Redirect URI: `https://your-docbrain-host/api/v1/auth/oidc/callback`
4. **Configure Application Credentials** step: generate a client secret.
5. **Web API** step: set the Identifier to the same callback URI.
6. **Apply Access Control Policy**: allow the appropriate users or groups.
7. **Configure Application Permissions**: add `openid`, `profile`, `email` scopes.
8. Add **Claim Rules** on the Issuance Transform Rules for the Web API:
   - Send LDAP attribute `E-Mail-Addresses` as claim type `email`
   - Send LDAP attribute `Display-Name` as claim type `name`
   - Send LDAP attribute `objectGUID` (or `objectSid`) as claim type `sub`

```env
OIDC_ISSUER_URL=https://<ADFS_HOST>/adfs
OIDC_CLIENT_ID=<CLIENT_IDENTIFIER>
OIDC_CLIENT_SECRET=<CLIENT_SECRET>
OIDC_REDIRECT_URI=https://your-docbrain-host/api/v1/auth/oidc/callback
OIDC_WEB_UI_URL=https://your-web-ui-host
```

Verify ADFS discovery is reachable before starting DocBrain:

```bash
curl https://<ADFS_HOST>/adfs/.well-known/openid-configuration
```

> **Note:** ADFS requires the redirect URI to be registered exactly as-is. A trailing slash difference will cause a redirect mismatch error.

---

## RBAC Role Mapping

By default, new SSO users are created with the `viewer` role. You can automatically upgrade users to `editor` or `admin` based on their email address or IdP group membership.

### Configuration

```env
OIDC_DEFAULT_ROLE=viewer            # viewer | editor | admin (default: viewer)
OIDC_ADMIN_DOMAIN=company.com       # All users with this email domain → admin
OIDC_ADMIN_EMAILS=alice@x.com,bob@y.com  # Specific emails → admin
OIDC_ADMIN_GROUPS=admins,platform   # IdP group claims → admin
OIDC_EDITOR_GROUPS=editors,writers  # IdP group claims → editor
```

### Priority order (highest wins)

1. Email is in `OIDC_ADMIN_EMAILS` → **admin**
2. Email domain matches `OIDC_ADMIN_DOMAIN` → **admin**
3. User's groups overlap with `OIDC_ADMIN_GROUPS` → **admin**
4. User's groups overlap with `OIDC_EDITOR_GROUPS` → **editor**
5. `OIDC_DEFAULT_ROLE` → default

### Role updates on re-login

Roles are **re-evaluated on every login**. If you remove a user from an admin group in your IdP, they will be downgraded to the appropriate role on their next login — no manual DB intervention needed.

> **Note:** Group claims are only evaluated when the IdP includes a `groups` claim in the ID token. Most providers require explicit configuration to include group membership. GitHub OAuth does not provide group claims.

### Example: all `@mycompany.com` employees get editor, engineering leads get admin

```env
OIDC_DEFAULT_ROLE=editor
OIDC_ADMIN_DOMAIN=mycompany.com
OIDC_ADMIN_GROUPS=engineering-leads,platform-team
```

---

## Multi-provider Login UI

The web UI automatically fetches `GET /api/v1/auth/providers` on load and renders only the buttons for configured providers. No frontend rebuilds required — adding `GITHUB_CLIENT_ID` to your env is enough to make the GitHub button appear.

```
GET /api/v1/auth/providers
→ { "oidc": true, "github": false, "gitlab": true }
```

---

## Security Notes

- **PKCE** (S256) is enforced on all OIDC flows (generic and GitLab). State tokens are stored in Redis with a 5-minute TTL and deleted on use.
- **Signature verification** is required on all OIDC ID tokens. The insecure fallback (`insecure_disable_signature_check`) is not available.
- **CSRF protection** on GitHub OAuth uses a random UUID state token stored in Redis (same TTL/delete pattern).
- **OTP exchange:** After any SSO callback, the server redirects to `OIDC_WEB_UI_URL?code=otp_<uuid>` instead of embedding the session key in the URL. The frontend exchanges this OTP via `POST /api/v1/auth/exchange` to receive the actual token in the response body — keeping raw API keys out of browser history, server access logs, and referrer headers. The OTP is single-use and expires after 60 seconds.
- **CLI OAuth** uses the same OTP pattern: the CLI starts a local HTTP server on a random port and the browser redirects to `http://127.0.0.1:{port}?code=otp_...`. The CLI exchanges the OTP to obtain and save the session key.
- **Session keys** created via SSO have the same TTL as email/password sessions (`LOGIN_SESSION_TTL_HOURS`).
- All SSO routes are public (no API key required) but all other routes remain authenticated.

---

## Troubleshooting

### "OIDC provider discovery failed"
The server cannot reach the `OIDC_ISSUER_URL`. Check network access from your DocBrain host to the IdP. The discovery endpoint is `{issuer_url}/.well-known/openid-configuration`.

### "ID token signature verification failed"
The provider returned a token that failed signature verification. Common causes:
- Clock skew > 5 minutes between DocBrain host and IdP
- Misconfigured `OIDC_ISSUER_URL` (must match the `iss` claim exactly)

### "Invalid or expired OIDC state"
The user took more than 5 minutes to complete the IdP login, or the state was already consumed. Ask them to try again.

### "Could not determine a verified email from GitHub"
The user's GitHub account has no verified primary email, or their email is set to private and the app doesn't have `user:email` scope. Check that your GitHub OAuth App requests `user:email` scope (DocBrain sets this automatically).

### GitHub button not appearing in UI
Check that both `GITHUB_CLIENT_ID` and `GITHUB_CLIENT_SECRET` are set. The providers endpoint returns `github: false` if either is missing.
