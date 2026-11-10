---
expect:
  file_paths: array
---
One capture exists for deploy/ingress/staging.yaml (procedure, indexed 2 days ago): "Uploads over 1 MB to the staging API failed with HTTP 413 until the ingress allowed a larger body. Set nginx.ingress.kubernetes.io/proxy-body-size: \"50m\" under metadata.annotations; no pod restart needed; verify with curl -F file=@2mb.bin … returns 200." Premise: deploy/ingress/staging.yaml.
