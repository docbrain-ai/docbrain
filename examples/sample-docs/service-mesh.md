# Istio Service Mesh Operations

## Overview

Acme Platform uses Istio 1.22 as our service mesh layer. It provides mTLS between all services, traffic management (canary, mirroring, fault injection), and observability (distributed tracing, metrics).

## Architecture

```
                    Istio Control Plane (istiod)
                    ┌──────────────────────────┐
                    │  Pilot  Citadel  Galley   │
                    └─────────┬────────────────┘
                              │ config push
              ┌───────────────┼───────────────┐
              ▼               ▼               ▼
        ┌──────────┐   ┌──────────┐   ┌──────────┐
        │ Service A │   │ Service B │   │ Service C │
        │ ┌──────┐ │   │ ┌──────┐ │   │ ┌──────┐ │
        │ │Envoy │ │   │ │Envoy │ │   │ │Envoy │ │
        │ │Proxy │ │   │ │Proxy │ │   │ │Proxy │ │
        │ └──────┘ │   │ └──────┘ │   │ └──────┘ │
        └──────────┘   └──────────┘   └──────────┘
```

## Common Operations

### Canary Deployments with Istio

```yaml
apiVersion: networking.istio.io/v1beta1
kind: VirtualService
metadata:
  name: payments-api
spec:
  hosts:
    - payments-api
  http:
    - match:
        - headers:
            x-canary:
              exact: "true"
      route:
        - destination:
            host: payments-api
            subset: canary
    - route:
        - destination:
            host: payments-api
            subset: stable
          weight: 90
        - destination:
            host: payments-api
            subset: canary
          weight: 10
```

### Traffic Mirroring

Mirror production traffic to a shadow environment for testing:

```yaml
apiVersion: networking.istio.io/v1beta1
kind: VirtualService
metadata:
  name: orders-api
spec:
  hosts:
    - orders-api
  http:
    - route:
        - destination:
            host: orders-api
            subset: stable
      mirror:
        host: orders-api-shadow
      mirrorPercentage:
        value: 100.0
```

### Fault Injection (Chaos Testing)

```yaml
apiVersion: networking.istio.io/v1beta1
kind: VirtualService
metadata:
  name: inventory-api
spec:
  hosts:
    - inventory-api
  http:
    - fault:
        delay:
          percentage:
            value: 10
          fixedDelay: 5s
        abort:
          percentage:
            value: 5
          httpStatus: 503
      route:
        - destination:
            host: inventory-api
```

## mTLS Configuration

All inter-service communication uses mTLS by default:

```yaml
apiVersion: security.istio.io/v1beta1
kind: PeerAuthentication
metadata:
  name: default
  namespace: istio-system
spec:
  mtls:
    mode: STRICT
```

To check mTLS status:
```bash
istioctl x describe pod $POD_NAME -n $NAMESPACE
```

## Troubleshooting

### "upstream connect error" or "503 UH"
1. Check if the destination pod is running: `kubectl get pods -n $NS`
2. Verify the DestinationRule exists: `kubectl get dr -n $NS`
3. Check Envoy config: `istioctl proxy-config cluster $POD -n $NS`

### High Latency via Mesh
1. Check Envoy access logs: `kubectl logs $POD -c istio-proxy --tail=100`
2. Review Kiali service graph for bottlenecks
3. Verify circuit breaker settings aren't too aggressive

## Team Contacts

- Service mesh: @platform-mesh in #infrastructure
- Platform team lead: Priya Patel (@priya)
