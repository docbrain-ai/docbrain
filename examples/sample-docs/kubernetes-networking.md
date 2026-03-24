# Kubernetes Networking Guide

## Overview

Acme Platform runs on EKS (us-east-1, us-west-2) with Istio service mesh for inter-service communication. All production traffic flows through the Istio ingress gateway, and mTLS is enforced between all pods.

## Service Discovery

Services register automatically via Kubernetes DNS:
```
<service-name>.<namespace>.svc.cluster.local
```

Example: The payments service is reachable at:
```
payments-api.payments.svc.cluster.local:8080
```

## Ingress Architecture

```
Internet → CloudFront → ALB → Istio IngressGateway → VirtualService → Pod
```

### Configuring a New Ingress Route

1. Create a `VirtualService` manifest:
```yaml
apiVersion: networking.istio.io/v1beta1
kind: VirtualService
metadata:
  name: my-service
  namespace: platform
spec:
  hosts:
    - api.acme-platform.com
  gateways:
    - istio-system/main-gateway
  http:
    - match:
        - uri:
            prefix: /api/v2/my-service
      route:
        - destination:
            host: my-service.platform.svc.cluster.local
            port:
              number: 8080
      timeout: 30s
      retries:
        attempts: 3
        retryOn: 5xx,reset,connect-failure
```

2. Apply: `kubectl apply -f virtualservice.yaml`
3. Verify: `istioctl analyze -n platform`

## Network Policies

All namespaces use deny-all default policies. Explicitly allow traffic:

```yaml
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: allow-payments-to-orders
  namespace: orders
spec:
  podSelector:
    matchLabels:
      app: orders-api
  ingress:
    - from:
        - namespaceSelector:
            matchLabels:
              name: payments
          podSelector:
            matchLabels:
              app: payments-api
      ports:
        - port: 8080
```

## DNS Resolution Issues

### Symptoms
- `SERVFAIL` or `NXDOMAIN` for cluster-internal names
- Intermittent `connection refused` errors

### Troubleshooting
1. Check CoreDNS pods: `kubectl get pods -n kube-system -l k8s-app=kube-dns`
2. Test resolution from the pod: `kubectl exec -it $POD -- nslookup payments-api.payments`
3. Check CoreDNS logs: `kubectl logs -n kube-system -l k8s-app=kube-dns --tail=100`
4. Verify `ndots` setting: `kubectl exec -it $POD -- cat /etc/resolv.conf`

### Common Fix
If resolution is slow, reduce `ndots` in your deployment:
```yaml
spec:
  dnsConfig:
    options:
      - name: ndots
        value: "2"
```

## Load Balancing

| Strategy | Use Case | Config |
|----------|----------|--------|
| Round Robin | Default | No config needed |
| Least Connections | Long-lived connections (WebSocket) | `trafficPolicy.loadBalancer.simple: LEAST_CONN` |
| Consistent Hash | Session affinity | `trafficPolicy.loadBalancer.consistentHash.httpHeaderName: x-user-id` |

## TLS Certificates

- Production: AWS ACM certificates, auto-renewed
- Internal: Istio CA issues short-lived certs (24h TTL)
- Staging: Let's Encrypt via cert-manager

## Escalation

For networking issues affecting production:
1. Page `@network-oncall` in #infrastructure
2. Check the Istio dashboard in Grafana
3. Escalate to the Platform team lead (Priya Patel)
