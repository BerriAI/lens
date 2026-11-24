{{- define "lens.deployment" -}}
{{- if and .Values.agents.enabled (ne (int .Values.replicaCount) 1) -}}
{{- fail "The agent workspace currently requires replicaCount=1 for its SQLite state and native bot ownership" -}}
{{- end -}}
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "lens.fullname" . }}
  {{- include "lens.annotations" (dict "root" .) | nindent 2 }}
  labels:
    {{- include "lens.labels" . | nindent 4 }}
spec:
  replicas: {{ .Values.replicaCount }}
  {{- if .Values.agents.enabled }}
  strategy:
    type: Recreate
  {{- end }}
  selector:
    matchLabels:
      {{- include "lens.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels:
        {{- include "lens.labels" . | nindent 8 }}
    spec:
      automountServiceAccountToken: false
      {{- with .Values.imagePullSecrets }}
      imagePullSecrets:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      securityContext:
        runAsNonRoot: true
        runAsUser: 65532
        runAsGroup: 65532
        fsGroup: 65532
        seccompProfile:
          type: RuntimeDefault
      {{- if include "lens.bundledClickhouse" . }}
      initContainers:
        - name: wait-for-clickhouse
          image: {{ .Values.clickhouse.image | quote }}
          command:
            - sh
            - -c
            - until wget -q -T 3 -O /dev/null "http://$CLICKHOUSE_HOST:8123/ping"; do sleep 2; done
          env:
            - name: CLICKHOUSE_HOST
              value: {{ include "lens.clickhouseName" . | quote }}
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: [ALL]
          resources:
            requests:
              cpu: 10m
              memory: 16Mi
            limits:
              memory: 64Mi
      {{- end }}
      containers:
        - name: {{ .Values.component }}
          image: {{ include "lens.image" . | quote }}
          imagePullPolicy: {{ .Values.image.pullPolicy }}
          securityContext:
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: [ALL]
          env:
            {{- if .Values.agents.enabled }}
            - name: LENS_AGENT_DATABASE
              value: /var/lib/lens/agents.db
            - name: LENS_RUNTIME_URL
              value: {{ printf "http://%s-agents:4320" (include "lens.fullname" .) | quote }}
            - name: LENS_RUNTIME_LENS_URL
              value: {{ printf "http://%s:%v/agent-runtime" (include "lens.fullname" .) .Values.service.port | quote }}
            - name: LENS_RUNTIME_SECRET
              valueFrom:
                secretKeyRef:
                  name: {{ include "lens.runtimeSecretName" . | quote }}
                  key: {{ .Values.agents.runtimeSecret.key | quote }}
            {{- end }}
            - name: LENS_MODE
              value: standalone
            - name: LENS_PUBLIC_URL
              value: {{ include "lens.publicUrl" . | quote }}
            - name: LITELLM_LENS_PUBLIC_URL
              value: {{ .Values.ingestionUrl | default (include "lens.publicUrl" .) | quote }}
            - name: LENS_ADMIN_TOKEN
              valueFrom:
                secretKeyRef:
                  name: {{ include "lens.adminTokenSecretName" . | quote }}
                  key: {{ .Values.adminTokenSecret.key | quote }}
            {{- if .Values.gateway.enabled }}
            - name: LENS_GATEWAY_SECRET
              valueFrom:
                secretKeyRef:
                  name: {{ include "lens.gatewaySecretName" . | quote }}
                  key: {{ .Values.gateway.secretKey | quote }}
            - name: LITELLM_LENS_SERVICE_TOKEN
              valueFrom:
                secretKeyRef:
                  name: {{ include "lens.serviceTokenSecretName" . | quote }}
                  key: {{ .Values.serviceTokenSecret.key | quote }}
            {{- end }}
            {{- if include "lens.bundledClickhouse" . }}
            - name: CLICKHOUSE_HOST
              value: {{ include "lens.clickhouseName" . | quote }}
            - name: CLICKHOUSE_USER
              value: {{ .Values.clickhouse.user | quote }}
            - name: CLICKHOUSE_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: {{ include "lens.clickhouseName" . | quote }}
                  key: password
            {{- else }}
            - name: CLICKHOUSE_URL
              valueFrom:
                secretKeyRef:
                  name: {{ required "Lens clickhouseSecret.name is required when bundled storage is disabled" .Values.clickhouseSecret.name | quote }}
                  key: {{ .Values.clickhouseSecret.key | quote }}
            {{- end }}
            - name: CLICKHOUSE_DATABASE
              value: {{ .Values.clickhouseDatabase | quote }}
            - name: AGENT_TRACING_RETENTION_DAYS
              value: {{ .Values.retentionDays | quote }}
            {{- with .Values.extraEnv }}
            {{- toYaml . | nindent 12 }}
            {{- end }}
          {{- with .Values.extraEnvFrom }}
          envFrom:
            {{- toYaml . | nindent 12 }}
          {{- end }}
          ports:
            - name: otlp
              containerPort: 4318
          livenessProbe:
            httpGet:
              path: /health/live
              port: otlp
          readinessProbe:
            httpGet:
              path: /health/ready
              port: otlp
          startupProbe:
            httpGet:
              path: /health/ready
              port: otlp
            periodSeconds: 5
            failureThreshold: 60
          resources:
            {{- toYaml .Values.resources | nindent 12 }}
          volumeMounts:
            {{- if .Values.agents.enabled }}
            - name: state
              mountPath: /var/lib/lens
            {{- end }}
            - name: tmp
              mountPath: /tmp
      volumes:
        {{- if .Values.agents.enabled }}
        - name: state
          persistentVolumeClaim:
            claimName: {{ include "lens.fullname" . }}-state
        {{- end }}
        - name: tmp
          emptyDir:
            medium: Memory
            sizeLimit: {{ .Values.tmpSizeLimit }}
      {{- with .Values.nodeSelector }}
      nodeSelector:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.tolerations }}
      tolerations:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      {{- with .Values.affinity }}
      affinity:
        {{- toYaml . | nindent 8 }}
      {{- end }}
{{- end -}}
