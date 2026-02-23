# Incident Response Playbook

## Severity Levels

| Level | Description | Response Time | Examples |
|-------|-------------|---------------|----------|
| SEV-1 | Complete service outage | 15 minutes | API down, data loss |
| SEV-2 | Major feature degraded | 30 minutes | Search broken, auth failing for subset |
| SEV-3 | Minor impact | 4 hours | Slow queries, UI glitch |
| SEV-4 | No user impact | Next business day | Internal tooling issue |

## SEV-1 Response Procedure

### 1. Acknowledge (first 5 minutes)
- Join #incident-response on Slack
- Claim the incident: `/incident claim SEV-1 <brief description>`
- Page the on-call engineer if not already paged

### 2. Assess (5-15 minutes)
- Check the status dashboard: https://status.acme-platform.internal
- Review recent deployments: `acme deploy history --since 2h`
- Check infrastructure metrics in Grafana

### 3. Mitigate (15-60 minutes)
Priority is **restoring service**, not finding root cause.

Common mitigations:
- **Bad deploy**: `acme deploy rollback <service>`
- **Database overload**: Enable read replicas, kill long-running queries
- **Traffic spike**: Scale up: `acme scale <service> --replicas 20`
- **Dependency failure**: Enable circuit breaker: `acme circuit-breaker enable <service>`

### 4. Communicate
- Update status page every 15 minutes
- Post to #engineering with current status
- If customer-facing: notify Customer Success team

### 5. Resolve and Retrospect
- Mark incident resolved: `/incident resolve`
- Schedule postmortem within 48 hours
- File follow-up tickets for permanent fixes

## Runbook: Database Connection Pool Exhaustion

**Symptoms**: Increasing error rate, "connection pool exhausted" in logs

**Steps**:
1. Check active connections: `SELECT count(*) FROM pg_stat_activity;`
2. Identify long-running queries: `SELECT pid, query, state, age(clock_timestamp(), query_start) FROM pg_stat_activity WHERE state != 'idle' ORDER BY query_start;`
3. Kill stuck queries: `SELECT pg_terminate_backend(pid);`
4. If persistent, increase pool size in config and redeploy

## Runbook: OpenSearch Cluster Yellow/Red

**Symptoms**: Search degraded or unavailable, cluster health yellow or red

**Steps**:
1. Check cluster health: `curl localhost:9200/_cluster/health?pretty`
2. Check disk space: `curl localhost:9200/_cat/allocation?v`
3. If disk full: delete old indices: `curl -X DELETE localhost:9200/logs-2024.01.*`
4. If shard allocation stuck: `curl -X POST localhost:9200/_cluster/reroute?retry_failed=true`

## Escalation Path

1. On-call engineer (PagerDuty)
2. Team lead
3. VP Engineering
4. CTO (SEV-1 only, if unresolved after 1 hour)
