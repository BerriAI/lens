{{- define "lens.secret" -}}
{{- $existing := lookup "v1" "Secret" .context.Release.Namespace .name }}
apiVersion: v1
kind: Secret
metadata:
  name: {{ .name }}
  annotations:
    helm.sh/resource-policy: keep
type: Opaque
data:
  {{ .key }}: {{ if $existing }}{{ required (printf "Saved Lens secret %s is missing key %s" .name .key) (index $existing.data .key) | quote }}{{ else }}{{ randAlphaNum 64 | b64enc | quote }}{{ end }}
{{- end -}}

{{- define "lens.secrets" -}}
{{- if and .Values.gateway.enabled (not .Values.serviceTokenSecret.name) }}
{{ include "lens.secret" (dict "context" . "name" (include "lens.serviceTokenSecretName" .) "key" .Values.serviceTokenSecret.key) }}
---
{{- end }}
{{- if include "lens.bundledClickhouse" . }}
{{ include "lens.secret" (dict "context" . "name" (include "lens.clickhouseName" .) "key" "password") }}
{{- end }}
---
{{- if not .Values.adminTokenSecret.name }}
{{ include "lens.secret" (dict "context" . "name" (include "lens.adminTokenSecretName" .) "key" .Values.adminTokenSecret.key) }}
{{- end }}
{{- end -}}
