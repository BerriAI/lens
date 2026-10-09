{{- define "lens.service" -}}
apiVersion: v1
kind: Service
metadata:
  name: {{ include "lens.fullname" . }}
  {{- include "lens.annotations" (dict "root" . "annotations" .Values.service.annotations) | nindent 2 }}
spec:
  selector:
    {{- include "lens.selectorLabels" . | nindent 4 }}
  ports:
    - name: otlp
      port: {{ .Values.service.port }}
      targetPort: otlp
{{- end -}}
