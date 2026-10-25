{{/* Base name for all objects. */}}
{{- define "ferrogate.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 50 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 50 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "ferrogate.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
app.kubernetes.io/name: ferrogate
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/* Selector labels for one component: include with (list . "operator"). */}}
{{- define "ferrogate.selector" -}}
app.kubernetes.io/name: ferrogate
app.kubernetes.io/instance: {{ (index . 0).Release.Name }}
app.kubernetes.io/component: {{ index . 1 }}
{{- end -}}

{{- define "ferrogate.image" -}}
{{- if .Values.image.digest -}}
{{ .Values.image.repository }}@{{ .Values.image.digest }}
{{- else -}}
{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}
{{- end -}}
{{- end -}}

{{- define "ferrogate.webhookService" -}}
{{ include "ferrogate.fullname" . }}-webhook
{{- end -}}

{{/* Restricted Pod Security settings shared by both containers. */}}
{{- define "ferrogate.containerSecurity" -}}
securityContext:
  allowPrivilegeEscalation: false
  readOnlyRootFilesystem: true
  runAsNonRoot: true
  capabilities: {drop: [ALL]}
{{- end -}}

{{- define "ferrogate.podSecurity" -}}
securityContext:
  runAsNonRoot: true
  seccompProfile: {type: RuntimeDefault}
{{- end -}}
