#!/usr/bin/env bash
set -euo pipefail
mkdir -p deploy/ingress
cat > deploy/ingress/staging.yaml <<'Y'
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: api
  namespace: staging
  annotations:
    kubernetes.io/ingress.class: nginx
spec:
  rules:
    - host: api.staging.example.test
      http:
        paths:
          - path: /
            pathType: Prefix
            backend: { service: { name: api, port: { number: 8080 } } }
Y
git init -q && git add -A && git -c user.name=eval -c user.email=eval@example.test commit -qm "staging ingress"
