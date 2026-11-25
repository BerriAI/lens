{{- define "lens.fullname" -}}
{{- .Values.fullnameOverride | default .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "lens.clickhouseName" -}}
{{- .Values.clickhouse.nameOverride | default (printf "%s-clickhouse" (include "lens.fullname" . | trunc 52 | trimSuffix "-")) -}}
{{- end -}}

{{- define "lens.clickhousePasswordSecretName" -}}
{{- .Values.clickhouse.passwordSecret.name | default (include "lens.clickhouseName" .) -}}
{{- end -}}

{{- define "lens.adminTokenSecretName" -}}
{{- .Values.adminTokenSecret.name | default (printf "%s-admin" (include "lens.fullname" . | trunc 57 | trimSuffix "-")) -}}
{{- end -}}

{{- define "lens.serviceTokenSecretName" -}}
{{- .Values.serviceTokenSecret.name | default .Values.serviceTokenSecret.generatedName | default (printf "%s-service" (include "lens.fullname" . | trunc 55 | trimSuffix "-")) -}}
{{- end -}}

{{- define "lens.gatewaySecretName" -}}
{{- .Values.gateway.secretName | default .Values.gateway.generatedName | default (printf "%s-gateway" (include "lens.fullname" . | trunc 55 | trimSuffix "-")) -}}
{{- end -}}

{{- define "lens.bundledClickhouse" -}}
{{- if and .Values.clickhouse.enabled (not .Values.clickhouseSecret.name) -}}true{{- end -}}
{{- end -}}

{{- define "lens.publicUrl" -}}
{{- if not .Values.ingress.enabled -}}
{{- .Values.publicUrl | default "http://localhost:4318" -}}
{{- else -}}
{{- $host := required "Lens ingress.host is required" .Values.ingress.host -}}
{{- if not .Values.publicUrl -}}
{{- printf "%s://%s" (ternary "https" "http" (not (empty .Values.ingress.tls))) $host -}}
{{- else -}}
{{- if ne (lower (urlParse .Values.publicUrl).hostname) (lower $host) -}}
{{- fail (printf "Lens publicUrl %s does not match ingress.host %s. Browsers on the ingress host could not sign in or save changes. Unset publicUrl to use the ingress host, or set it to the address browsers open" .Values.publicUrl $host) -}}
{{- end -}}
{{- .Values.publicUrl -}}
{{- end -}}
{{- end -}}
{{- end -}}

{{- define "lens.image" -}}
{{- if .Values.image.digest -}}
{{- if not (regexMatch "^sha256:[0-9a-f]{64}$" .Values.image.digest) -}}
{{- fail "Lens image.digest must be sha256 followed by 64 lowercase hex characters" -}}
{{- end -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else -}}
{{- printf "%s:%s" .Values.image.repository (.Values.image.tag | default .Chart.AppVersion) -}}
{{- end -}}
{{- end -}}

{{- define "lens.selectorLabels" -}}
app.kubernetes.io/instance: {{ .Values.instanceOverride | default .Release.Name }}
app.kubernetes.io/component: {{ .Values.component }}
{{- end -}}

{{- define "lens.annotations" -}}
{{- $annotations := deepCopy (.annotations | default dict) -}}
{{- if .root.Values.retainResources -}}
{{- $_ := set $annotations "helm.sh/resource-policy" "keep" -}}
{{- end -}}
{{- with $annotations }}
annotations:
  {{- toYaml . | nindent 2 }}
{{- end -}}
{{- end -}}

{{- define "lens.labels" -}}
{{ include "lens.selectorLabels" . }}
app.kubernetes.io/name: {{ .Values.nameOverride }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}
