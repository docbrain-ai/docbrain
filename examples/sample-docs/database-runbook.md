# PostgreSQL Database Operations Runbook

## Overview

Acme Platform uses Amazon RDS PostgreSQL 16.2 for all persistent storage. We run Multi-AZ deployments in us-east-1 with read replicas for analytics workloads. This runbook covers common operational procedures and incident response.

## Connection Details

| Environment | Endpoint | Port | Database |
|-------------|----------|------|----------|
| Production | rds-prod.acme-platform.internal | 5432 | acme_prod |
| Staging | rds-staging.acme-platform.internal | 5432 | acme_staging |
| Read Replica | rds-replica.acme-platform.internal | 5432 | acme_prod |

## Common Operations

### Running Migrations

```bash
# Check current migration status
acme db status --env production

# Run pending migrations (with automatic rollback on failure)
acme db migrate --env production

# Rollback the last migration
acme db rollback --env production --steps 1
```

**Important:** Always run migrations during the maintenance window (Tuesday 2-4 AM ET). Large schema changes must be reviewed by the database team first.

### Query Performance Investigation

```sql
-- Find slow queries (> 1 second)
SELECT pid, now() - pg_stat_activity.query_start AS duration,
       query, state
FROM pg_stat_activity
WHERE (now() - pg_stat_activity.query_start) > interval '1 second'
  AND state != 'idle'
ORDER BY duration DESC;

-- Check index usage
SELECT schemaname, tablename, indexname, idx_scan, idx_tup_read
FROM pg_stat_user_indexes
WHERE idx_scan = 0
ORDER BY pg_relation_size(indexrelid) DESC
LIMIT 20;

-- Table bloat estimation
SELECT schemaname, tablename,
       pg_size_pretty(pg_total_relation_size(schemaname || '.' || tablename)) as total_size,
       n_dead_tup,
       round(n_dead_tup::numeric / greatest(n_live_tup, 1) * 100, 2) as dead_pct
FROM pg_stat_user_tables
WHERE n_dead_tup > 10000
ORDER BY n_dead_tup DESC;
```

### Connection Pool Exhaustion

**Symptoms:** `too many connections` errors, increasing latency

**Resolution:**
1. Check active connections: `SELECT count(*) FROM pg_stat_activity;`
2. Identify idle-in-transaction connections:
   ```sql
   SELECT pid, usename, state, query_start, query
   FROM pg_stat_activity
   WHERE state = 'idle in transaction'
     AND query_start < now() - interval '5 minutes';
   ```
3. Kill idle connections:
   ```sql
   SELECT pg_terminate_backend(pid)
   FROM pg_stat_activity
   WHERE state = 'idle in transaction'
     AND query_start < now() - interval '10 minutes';
   ```
4. If the issue persists, increase `max_connections` in RDS parameter group

### Backup and Recovery

- **Automated backups:** Daily snapshots, 7-day retention
- **Point-in-time recovery:** Available to any second within the retention window
- **Manual snapshots:** Before major migrations, created via AWS Console

```bash
# Create manual snapshot
aws rds create-db-snapshot \
  --db-instance-identifier acme-prod \
  --db-snapshot-identifier pre-migration-$(date +%Y%m%d)

# Restore from snapshot (creates new instance)
aws rds restore-db-instance-from-db-snapshot \
  --db-instance-identifier acme-prod-restored \
  --db-snapshot-identifier pre-migration-20260315
```

## Incident Response

### Database Failover

If the primary instance fails, RDS automatically fails over to the standby:

1. Failover takes 60-120 seconds
2. Application connections will drop — connection pools retry automatically
3. DNS endpoint doesn't change, but the underlying IP does
4. Check failover status: `aws rds describe-events --source-identifier acme-prod --duration 60`

### Disk Space Emergency

1. Check current usage: `aws rds describe-db-instances --db-instance-identifier acme-prod --query 'DBInstances[0].FreeStorageSpace'`
2. Identify large tables: `SELECT pg_size_pretty(pg_total_relation_size(tablename::regclass)) FROM pg_tables ORDER BY pg_total_relation_size(tablename::regclass) DESC LIMIT 10;`
3. If needed, increase storage: `aws rds modify-db-instance --db-instance-identifier acme-prod --allocated-storage 500 --apply-immediately`

**Note:** RDS storage can only be increased, never decreased. Plan capacity accordingly.

## Team Contacts

| Role | Contact |
|------|---------|
| DBA on-call | @dba-oncall in #database |
| Database team lead | Marcus Rivera |
| AWS support | Enterprise support case |
