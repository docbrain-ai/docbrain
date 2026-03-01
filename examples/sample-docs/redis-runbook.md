# Redis OOM Troubleshooting Runbook

## Symptoms

- `OOM command not allowed when used memory > 'maxmemory'` in Redis logs
- Application errors: connection timeouts or `COMMAND_REJECTED` responses
- PagerDuty alert: `redis-memory-critical` (>90% of maxmemory)

## Immediate Response

### 1. Check Current Memory Usage

```bash
acme redis info memory --cluster production
# or directly:
redis-cli -h redis-prod.internal INFO memory
```

Key fields:
- `used_memory_human` — actual memory consumed
- `maxmemory_human` — configured limit
- `evicted_keys` — keys removed by eviction policy (should be 0 in normal operation)

### 2. Identify the Largest Keys

```bash
redis-cli -h redis-prod.internal --bigkeys --memkeys
```

Common culprits:
- Session caches that grew unbounded (check `session:*` pattern)
- Job queues with stuck/dead consumers (check `queue:*` pattern)
- Incorrectly serialized objects (check key sizes > 1MB)

### 3. Emergency Relief

If memory is >95% and eviction isn't helping:

```bash
# Option A: Flush non-critical caches (safe — they rebuild)
redis-cli -h redis-prod.internal EVAL "local keys = redis.call('keys', 'cache:*') for i,k in ipairs(keys) do redis.call('del', k) end return #keys" 0

# Option B: Increase maxmemory temporarily (buys time)
redis-cli -h redis-prod.internal CONFIG SET maxmemory 8gb
```

**Do NOT restart Redis** unless absolutely necessary — this causes a full cache cold-start.

## Root Cause Investigation

After immediate pressure is relieved:

1. Check if a recent deploy changed cache TTLs or introduced new key patterns
2. Review the `redis-memory` Grafana dashboard for the growth inflection point
3. Correlate with deploy timestamps: `acme deploy history --since 24h`

## Our Maxmemory Policy

Production clusters use `allkeys-lru` policy. This means:
- When memory hits the limit, Redis evicts least-recently-used keys automatically
- If OOM still occurs, it means write pressure exceeds eviction throughput
- Solution: either reduce write volume or increase maxmemory

## Prevention

- All cache keys MUST have a TTL (`SETEX`, not `SET`)
- Maximum TTL for caches: 24 hours (enforced by the caching library)
- Session data: 30-minute sliding window
- Queue consumers must acknowledge within 60 seconds or the job is re-queued

## Escalation

If the above doesn't resolve within 15 minutes:
1. Page `@redis-oncall` in #infrastructure
2. If no response in 10 minutes, escalate to the Platform Lead
