# API Authentication & Authorization

## Overview

Acme Platform uses OAuth 2.0 with JWT tokens for all API authentication. Every request to a protected endpoint must include a valid access token in the `Authorization` header. Tokens are issued by the Identity Service and validated by the API Gateway.

## Authentication Flow

### 1. Service-to-Service (M2M)

For backend services communicating with each other:

```bash
curl -X POST https://auth.acme-platform.internal/oauth/token \
  -H "Content-Type: application/json" \
  -d '{
    "grant_type": "client_credentials",
    "client_id": "payments-service",
    "client_secret": "$CLIENT_SECRET",
    "audience": "https://api.acme-platform.internal"
  }'
```

Response:
```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIs...",
  "token_type": "Bearer",
  "expires_in": 3600,
  "scope": "read:orders write:payments"
}
```

### 2. User Authentication (PKCE)

For frontend applications and CLIs:

```bash
# Step 1: Generate code verifier and challenge
CODE_VERIFIER=$(openssl rand -base64 32 | tr -d '=/+')
CODE_CHALLENGE=$(echo -n "$CODE_VERIFIER" | openssl dgst -sha256 -binary | base64 | tr -d '=/+')

# Step 2: Redirect user to authorize
https://auth.acme-platform.internal/authorize?
  response_type=code&
  client_id=acme-cli&
  redirect_uri=http://localhost:9876/callback&
  code_challenge=$CODE_CHALLENGE&
  code_challenge_method=S256

# Step 3: Exchange code for token
curl -X POST https://auth.acme-platform.internal/oauth/token \
  -d "grant_type=authorization_code&code=$AUTH_CODE&code_verifier=$CODE_VERIFIER"
```

## Token Validation

All services validate tokens using the Identity Service's JWKS endpoint:

```
GET https://auth.acme-platform.internal/.well-known/jwks.json
```

Token claims structure:
```json
{
  "sub": "user:alice@acme.com",
  "aud": "https://api.acme-platform.internal",
  "iss": "https://auth.acme-platform.internal",
  "exp": 1711234567,
  "scope": "read:all write:deployments",
  "teams": ["platform", "payments"],
  "role": "engineer"
}
```

## Rate Limiting by Auth Tier

| Auth Type | Rate Limit | Burst | Notes |
|-----------|-----------|-------|-------|
| Service Account | 10,000/min | 500 | Per client_id |
| User Token | 1,000/min | 100 | Per user |
| API Key (legacy) | 100/min | 20 | Deprecated Q2 2026 |

## API Key Migration

Legacy API keys (`ak_*` prefix) are being deprecated. Migration steps:

1. Generate a new service account in the Admin Console
2. Update your service to use OAuth 2.0 client credentials flow
3. Test with `ACME_AUTH_MODE=oauth2` environment variable
4. Remove the old API key from your configuration
5. Delete the API key in the Admin Console

**Deadline:** June 30, 2026. After this date, `ak_*` keys will stop working.

## Troubleshooting

### "Token expired" errors
Tokens expire after 1 hour. Implement token refresh:
```python
def get_token():
    if cached_token and cached_token.exp > time.now() + 300:
        return cached_token
    return refresh_token()
```

### "Insufficient scope" errors
Check that your service account has the required scopes. Request additional scopes via the Admin Console under **Settings > Service Accounts > Scopes**.

### "JWKS fetch failed" errors
The Identity Service caches JWKS for 24 hours. If you rotated keys recently, wait for the cache to expire or restart the service.

## Security Contacts

- Security team: #security on Slack
- Incident response: security@acme-platform.internal
- Key rotation schedule: First Monday of every month
