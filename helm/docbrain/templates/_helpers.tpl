{{- define "docbrain.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "docbrain.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "docbrain.labels" -}}
helm.sh/chart: {{ .Chart.Name }}-{{ .Chart.Version }}
app.kubernetes.io/name: {{ include "docbrain.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "docbrain.selectorLabels" -}}
app.kubernetes.io/name: {{ include "docbrain.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "docbrain.secretName" -}}
{{- if .Values.existingSecret }}
{{- .Values.existingSecret }}
{{- else }}
{{- include "docbrain.fullname" . }}-secret
{{- end }}
{{- end }}

{{/* Issue #1: Use -}} to strip trailing newlines so URL values don't start with \n */}}
{{- define "docbrain.databaseUrl" -}}
{{- if .Values.postgresql.internal -}}
postgresql://docbrain:$(POSTGRES_PASSWORD)@{{ include "docbrain.fullname" . }}-postgres:5432/docbrain
{{- else -}}
{{- .Values.postgresql.externalUrl }}
{{- end -}}
{{- end }}

{{- define "docbrain.opensearchUrl" -}}
{{- if .Values.opensearch.internal -}}
http://{{ include "docbrain.fullname" . }}-opensearch:9200
{{- else -}}
{{- .Values.opensearch.externalUrl }}
{{- end -}}
{{- end }}

{{- define "docbrain.redisUrl" -}}
{{- if .Values.redis.internal -}}
redis://{{ include "docbrain.fullname" . }}-redis:6379
{{- else -}}
{{- .Values.redis.externalUrl }}
{{- end -}}
{{- end }}

{{- define "docbrain.serviceAccountName" -}}
{{- if .Values.serviceAccount.create }}
{{- default (include "docbrain.fullname" .) .Values.serviceAccount.name }}
{{- else }}
{{- default "default" .Values.serviceAccount.name }}
{{- end }}
{{- end }}

{{/*
Pod-template checksums: a workload restarts on `helm upgrade` when, and only when, an object it reads
whole (envFrom or a mounted volume) changed. Each annotation is checksum/<suffix> for the object named
<fullname>-<suffix>. Add only the checksums of the objects the pod reads: a checksum of anything else
restarts the pod for nothing (the datastores used to restart on every server setting change).
Not checksummed, by design: the bootstrap admin key and the fingerprint key Secrets (generated once
and kept by lookup, so they never change on upgrade) and single keys read by secretKeyRef.
tests/workload_wiring_test.py holds this rule for every workload.

docbrain.configChecksum: the settings ConfigMap and, when the chart renders it, the Secret.
*/}}
{{- define "docbrain.configChecksum" -}}
checksum/config: {{ include (print $.Template.BasePath "/configmap.yaml") . | sha256sum }}
{{- if not .Values.existingSecret }}
checksum/secret: {{ include (print $.Template.BasePath "/secret.yaml") . | sha256sum }}
{{- end }}
{{- end }}

{{/* "true" when the chart renders the local.yaml overlay ConfigMap (<fullname>-sources), else empty. */}}
{{- define "docbrain.hasLocalOverlay" -}}
{{- $hasSources := and (kindIs "map" .Values.ingest.sources) (gt (len .Values.ingest.sources) 0) }}
{{- $hasExclusion := and (kindIs "map" .Values.freshness) (kindIs "map" .Values.freshness.exclusionRules) (gt (len .Values.freshness.exclusionRules) 0) }}
{{- $hasEventSpaces := and (kindIs "map" .Values.freshness) (kindIs "slice" .Values.freshness.eventBasedSpaces) (gt (len .Values.freshness.eventBasedSpaces) 0) }}
{{- if or $hasSources $hasExclusion $hasEventSpaces }}true{{ end }}
{{- end }}

{{/* Checksum of the local.yaml overlay, for the pods that mount it (only when it renders). */}}
{{- define "docbrain.localOverlayChecksum" -}}
{{- if include "docbrain.hasLocalOverlay" . }}
checksum/sources: {{ include (print $.Template.BasePath "/sources-configmap.yaml") . | sha256sum }}
{{- end }}
{{- end }}

{{/* The Secret (and its key) the server reads DOCBRAIN_FINGERPRINT_KEY from. */}}
{{- define "docbrain.fingerprintKeySecretName" -}}
{{- if .Values.fingerprintKeySecret.existingSecret }}
{{- .Values.fingerprintKeySecret.existingSecret }}
{{- else }}
{{- include "docbrain.fullname" . }}-fingerprint-key
{{- end }}
{{- end }}

{{- define "docbrain.fingerprintKeySecretKey" -}}
{{- if .Values.fingerprintKeySecret.existingSecret }}
{{- default "DOCBRAIN_FINGERPRINT_KEY" .Values.fingerprintKeySecret.key }}
{{- else }}
{{- "DOCBRAIN_FINGERPRINT_KEY" }}
{{- end }}
{{- end }}

{{- /* A model value as its reader must see it: trimmed, and "" when unset, empty or only whitespace. An empty model
       setting means the provider's default, so a template renders the key only when this is not "". */}}
{{- define "docbrain.modelId" -}}
{{- . | default "" | toString | trim -}}
{{- end }}

{{- /* The base model the server fine-tunes from (LEARNING_BASE_MODEL_ID); the trainer must get the same one. */}}
{{- define "docbrain.baseModelId" -}}
{{- include "docbrain.modelId" .Values.learning.training.baseModelId | default "sentence-transformers/all-MiniLM-L6-v2" -}}
{{- end }}
