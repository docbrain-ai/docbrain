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

{{/*
Validate required API keys are not left as placeholder values.
Called from configmap.yaml so it runs on every helm install/upgrade.
*/}}
{{- define "docbrain.validate" -}}
{{- if and (eq .Values.llm.provider "anthropic") (or (eq (.Values.llm.anthropicApiKey | default "") "CHANGE_ME") (empty (.Values.llm.anthropicApiKey | default ""))) -}}
{{- fail "ERROR: llm.anthropicApiKey must be set when llm.provider=anthropic. Pass it via --set llm.anthropicApiKey=<key>" -}}
{{- end -}}
{{- if and (eq .Values.llm.provider "openai") (or (eq (.Values.llm.openaiApiKey | default "") "CHANGE_ME") (empty (.Values.llm.openaiApiKey | default ""))) -}}
{{- fail "ERROR: llm.openaiApiKey must be set when llm.provider=openai. Pass it via --set llm.openaiApiKey=<key>" -}}
{{- end -}}
{{- if and (eq .Values.embedding.provider "openai") (or (eq (.Values.embedding.openaiApiKey | default .Values.llm.openaiApiKey | default "") "CHANGE_ME") (empty (.Values.embedding.openaiApiKey | default .Values.llm.openaiApiKey | default ""))) -}}
{{- fail "ERROR: embedding.openaiApiKey (or llm.openaiApiKey) must be set when embedding.provider=openai. Pass it via --set embedding.openaiApiKey=<key>" -}}
{{- end -}}
{{- end -}}

{{/*
Checksum of ConfigMap + Secret — used in pod annotations to trigger rolling restarts
when configuration changes. Append to pod template metadata.annotations.
*/}}
{{- define "docbrain.configChecksum" -}}
checksum/config: {{ include (print $.Template.BasePath "/configmap.yaml") . | sha256sum }}
checksum/secret: {{ include (print $.Template.BasePath "/secret.yaml") . | sha256sum }}
{{- end }}
