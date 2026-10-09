{{- define "lens.ingress" -}}
{{- if .Values.ingress.enabled }}
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: {{ include "lens.fullname" . }}
  {{- include "lens.annotations" (dict "root" . "annotations" .Values.ingress.annotations) | nindent 2 }}
spec:
  {{- with .Values.ingress.className }}
  ingressClassName: {{ . | quote }}
  {{- end }}
  {{- with .Values.ingress.tls }}
  tls:
    {{- toYaml . | nindent 4 }}
  {{- end }}
  rules:
    - host: {{ required "Lens ingress.host is required" .Values.ingress.host | quote }}
      http:
        paths:
          - path: {{ .Values.ingress.path | quote }}
            pathType: Prefix
            backend:
              service:
                name: {{ include "lens.fullname" . }}
                port:
                  name: otlp
{{- end }}
{{- end -}}
