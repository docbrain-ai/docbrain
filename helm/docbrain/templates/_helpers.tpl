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

{{- define "docbrain.databaseUrl" -}}
{{- if .Values.postgresql.internal }}
postgresql://docbrain:$(POSTGRES_PASSWORD)@{{ include "docbrain.fullname" . }}-postgres:5432/docbrain
{{- else }}
{{- .Values.postgresql.externalUrl }}
{{- end }}
{{- end }}

{{- define "docbrain.opensearchUrl" -}}
{{- if .Values.opensearch.internal }}
http://{{ include "docbrain.fullname" . }}-opensearch:9200
{{- else }}
{{- .Values.opensearch.externalUrl }}
{{- end }}
{{- end }}

{{- define "docbrain.redisUrl" -}}
{{- if .Values.redis.internal }}
redis://{{ include "docbrain.fullname" . }}-redis:6379
{{- else }}
{{- .Values.redis.externalUrl }}
{{- end }}
{{- end }}
