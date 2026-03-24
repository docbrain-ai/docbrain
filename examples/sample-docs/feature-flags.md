# Feature Flag System

## Overview

Acme Platform uses LaunchDarkly for feature flags. All new features must be launched behind a flag, and all flags must have an owner and an expiration date. The Feature Flag Committee reviews flag hygiene monthly.

## Creating a Feature Flag

### Naming Convention

```
<team>.<feature>.<variant>
```

Examples:
- `payments.refund-automation.enabled`
- `platform.new-deploy-ui.beta`
- `search.vector-search.rollout-pct`

### Steps

1. Create the flag in LaunchDarkly (UI or API)
2. Add the flag key to the service's flag manifest:
   ```yaml
   # flags.yaml
   flags:
     - key: payments.refund-automation.enabled
       owner: marcus@acme.com
       created: 2026-03-01
       expires: 2026-06-01
       description: "Automated refunds for orders under $50"
       default: false
   ```
3. Use the flag in code:
   ```python
   if ld_client.variation("payments.refund-automation.enabled", user, False):
       process_automated_refund(order)
   else:
       queue_for_manual_review(order)
   ```

## Rollout Strategies

### Percentage Rollout
Gradually increase traffic to the new feature:
- Day 1: 5% (internal employees)
- Day 3: 25% (if metrics stable)
- Day 7: 50%
- Day 14: 100% (flag removal candidate)

### Targeted Rollout
Enable for specific segments:
- Beta users
- Internal employees
- Specific customer accounts

### Kill Switch
Every production feature should have a kill switch flag that can disable it instantly without a deploy.

## Flag Lifecycle

| Phase | Duration | Action |
|-------|----------|--------|
| Created | - | Flag created, default OFF |
| Testing | 1-2 weeks | Enabled in staging |
| Rollout | 2-4 weeks | Gradual production rollout |
| Fully Rolled Out | - | Flag is ON for 100% |
| Cleanup | Within 30 days | Remove flag from code |
| Archived | - | Flag deleted from LaunchDarkly |

**Important:** Flags that are fully rolled out for more than 30 days must be cleaned up. The Feature Flag Committee will escalate unresolved stale flags.

## Monitoring Feature Flags

Dashboard: Grafana > Feature Flags

Key metrics:
- Flag evaluation rate per service
- Error rate with flag ON vs OFF
- Latency impact of flag-gated features

## Emergency Flag Toggle

During an incident, any on-call engineer can toggle a kill switch flag:

```bash
# Via CLI
acme flags toggle payments.refund-automation.enabled --off --reason "SEV-1 incident INC-456"

# Via LaunchDarkly UI
# Dashboard → Feature Flags → Search → Toggle OFF
```

All emergency toggles are audited and require a post-incident review.

## Contacts

- Feature Flag Committee: #feature-flags on Slack
- LaunchDarkly admin: Platform team
- Flag cleanup reports: Sent monthly to engineering managers
