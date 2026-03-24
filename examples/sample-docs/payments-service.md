# Payments Service Architecture

## Overview

The Payments Service processes all financial transactions for Acme Platform. It handles payment intents, charge authorization, capture, refunds, and dispute management. The service processes approximately 50,000 transactions per day across 12 currencies.

## Architecture

```
Client → API Gateway → Payments API → Payment Processor (Stripe/Adyen)
                                    ↓
                              PostgreSQL (ledger)
                                    ↓
                              Event Bus → Notifications, Analytics, Reconciliation
```

## Key Components

### Payment Intent Flow

1. **Create Intent** — Client submits payment details, amount, and currency
2. **Authorize** — Payment processor validates the card and reserves funds
3. **Capture** — Funds are captured after order fulfillment (up to 7 days)
4. **Settle** — Funds transfer to our merchant account (T+2 business days)

### Idempotency

All payment endpoints support idempotency keys:

```bash
curl -X POST https://api.acme-platform.internal/v1/payments/intents \
  -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: order-12345-attempt-1" \
  -H "Content-Type: application/json" \
  -d '{
    "amount": 4999,
    "currency": "usd",
    "payment_method": "pm_card_visa",
    "metadata": {"order_id": "order-12345"}
  }'
```

**Critical:** Always use deterministic idempotency keys. Random UUIDs defeat the purpose.

### Retry Logic

The payments service uses exponential backoff with jitter:

```rust
let delay = min(base_delay * 2^attempt + random_jitter, max_delay);
// base_delay: 100ms, max_delay: 30s, max_attempts: 5
```

Failed charges are retried up to 3 times before marking as `failed`. Retries only happen for transient errors (network timeouts, 503s). Card declines are never retried.

## Database Schema

The ledger uses double-entry bookkeeping:

```sql
-- Every transaction creates two entries
INSERT INTO ledger_entries (transaction_id, account, direction, amount, currency)
VALUES
  ($1, 'customer:alice', 'debit',  4999, 'usd'),
  ($1, 'revenue:sales',  'credit', 4999, 'usd');
```

## Refund Process

### Automatic Refunds (< $50)
- Customer self-service via the dashboard
- Processed within 5-10 business days
- No approval required

### Manual Refunds (>= $50)
- Requires finance team approval
- Submit via #finance-approvals Slack channel
- Include: order ID, amount, reason, customer email
- SLA: 24 hours for approval, 5-10 business days for processing

### Partial Refunds
```bash
curl -X POST https://api.acme-platform.internal/v1/payments/refunds \
  -H "Authorization: Bearer $TOKEN" \
  -d '{"payment_intent": "pi_xxx", "amount": 2500, "reason": "partial_return"}'
```

## Monitoring

| Metric | Alert Threshold | Dashboard |
|--------|----------------|-----------|
| Payment success rate | < 98% | Grafana: Payments Overview |
| Authorization latency (p99) | > 2s | Grafana: Payments Latency |
| Refund processing time | > 10 days | PagerDuty: payments-sla |
| Reconciliation drift | > $100 | PagerDuty: payments-recon |

## On-Call Responsibilities

The payments on-call engineer is responsible for:
- Monitoring transaction success rates
- Investigating payment processor outages
- Handling dispute escalations from the finance team
- Processing emergency refunds outside of business hours

**Escalation:** @payments-oncall → Marcus Rivera (Tech Lead) → VP Engineering
