# Monitoring & Alerting Guide

## Overview

Acme Platform uses a comprehensive observability stack: Prometheus for metrics, Grafana for dashboards, Loki for logs, and Tempo for distributed tracing. All alerts are routed through PagerDuty with Slack notifications.

## Observability Stack

| Component | Purpose | URL |
|-----------|---------|-----|
| Grafana | Dashboards & visualization | https://grafana.acme-platform.internal |
| Prometheus | Metrics collection & alerting | https://prometheus.acme-platform.internal |
| Loki | Log aggregation | (via Grafana) |
| Tempo | Distributed tracing | (via Grafana) |
| PagerDuty | Incident management | https://acme.pagerduty.com |

## Key Dashboards

### Service Health Dashboard
Shows real-time health of all services: request rate, error rate, latency percentiles, and saturation metrics.

**Location:** Grafana > Dashboards > Service Health

### Business Metrics
Tracks business KPIs: orders/minute, payment success rate, user signups, active sessions.

**Location:** Grafana > Dashboards > Business Metrics

### Infrastructure
Node CPU/memory, pod counts, disk usage, network throughput across all clusters.

**Location:** Grafana > Dashboards > Infrastructure

## Alert Tiers

| Tier | Response Time | Notification | Example |
|------|--------------|--------------|---------|
| P1 - Critical | 5 min | PagerDuty page + Slack | Service down, data loss risk |
| P2 - High | 15 min | PagerDuty + Slack | Error rate > 5%, latency > 5s |
| P3 - Medium | 1 hour | Slack only | Disk > 80%, memory > 85% |
| P4 - Low | Next business day | Slack only | Certificate expiring in 30 days |

## Creating Custom Alerts

### Prometheus Alert Rule

```yaml
groups:
  - name: payments
    rules:
      - alert: PaymentSuccessRateLow
        expr: |
          sum(rate(payment_requests_total{status="success"}[5m]))
          /
          sum(rate(payment_requests_total[5m]))
          < 0.98
        for: 5m
        labels:
          severity: critical
          team: payments
        annotations:
          summary: "Payment success rate below 98%"
          description: "Current rate: {{ $value | humanizePercentage }}"
          runbook: "https://wiki.acme-platform.internal/runbooks/payments-low-success-rate"
```

### Adding a New Dashboard Panel

1. Open Grafana → Select dashboard → Edit
2. Add new panel → Select data source (Prometheus/Loki)
3. Enter PromQL query
4. Configure visualization and thresholds
5. Save dashboard

## Standard Metrics

All services should expose these metrics via `/metrics`:

```
# Request metrics
http_requests_total{method, path, status}
http_request_duration_seconds{method, path}

# Application metrics
<service>_operations_total{operation, status}
<service>_operation_duration_seconds{operation}

# Resource metrics
process_cpu_seconds_total
process_resident_memory_bytes
```

## Log Standards

All services use structured JSON logging:

```json
{
  "timestamp": "2026-03-24T10:30:00Z",
  "level": "error",
  "service": "payments-api",
  "trace_id": "abc123",
  "span_id": "def456",
  "message": "Payment authorization failed",
  "error": "card_declined",
  "payment_intent": "pi_xxx",
  "duration_ms": 234
}
```

**Required fields:** timestamp, level, service, message, trace_id

## Distributed Tracing

All inter-service calls propagate trace context via `traceparent` header (W3C Trace Context):

```
traceparent: 00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01
```

To investigate a slow request:
1. Find the trace ID in logs or Grafana
2. Open Tempo → Search by trace ID
3. View the full request waterfall

## Escalation

- **Monitoring infrastructure issues:** #observability on Slack
- **Alert routing changes:** File a ticket with the Platform team
- **PagerDuty access:** Request via IT helpdesk
