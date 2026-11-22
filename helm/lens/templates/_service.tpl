{{- define "lens.service" -}}
apiVersion: v1
kind: Service
metadata:
  name: {{ include "lens.fullname" . }}
  {{- include "lens.annotations" (dict "root" . "annotations" .Values.service.annotations) | nindent 2 }}
spec:
  type: {{ .Values.service.type }}
  selector:
    {{- include "lens.selectorLabels" . | nindent 4 }}
  ports:
    - name: otlp
      port: {{ .Values.service.port }}
      targetPort: otlp
      {{- with .Values.service.nodePort }}
      nodePort: {{ . }}
      {{- end }}
{{- end -}}
