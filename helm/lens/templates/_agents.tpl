{{- define "lens.agents" -}}
{{- if .Values.agents.enabled }}
{{- $name := include "lens.fullname" . }}
{{- $instance := .Values.instanceOverride | default .Release.Name }}
{{- range $volume := list (dict "name" "state" "size" .Values.agents.sharedStorage) (dict "name" "native" "size" .Values.agents.nativeStorage) }}
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: {{ $name }}-{{ $volume.name }}
  annotations:
    helm.sh/resource-policy: keep
spec:
  accessModes: [ReadWriteOnce]
  {{- if ne $.Values.agents.storageClassName nil }}
  storageClassName: {{ $.Values.agents.storageClassName | quote }}
  {{- end }}
  resources:
    requests:
      storage: {{ $volume.size }}
---
{{- end }}
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ $name }}-agents
spec:
  replicas: 1
  strategy:
    type: Recreate
  selector:
    matchLabels:
      app.kubernetes.io/instance: {{ $instance }}
      app.kubernetes.io/component: lens-agents
  template:
    metadata:
      labels:
        app.kubernetes.io/instance: {{ $instance }}
        app.kubernetes.io/component: lens-agents
    spec:
      automountServiceAccountToken: false
      {{- with .Values.imagePullSecrets }}
      imagePullSecrets:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      securityContext:
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: agents
          image: {{ include "lens.image" (dict "Values" (dict "image" .Values.agents.image) "Chart" .Chart) | quote }}
          imagePullPolicy: {{ .Values.agents.image.pullPolicy }}
          securityContext:
            runAsUser: 0
            runAsGroup: 0
            allowPrivilegeEscalation: false
            readOnlyRootFilesystem: true
            capabilities:
              drop: [ALL]
              add: [CHOWN, FOWNER, SETUID, SETGID, DAC_OVERRIDE, KILL]
          env:
            - name: LENS_RUNTIME_SECRET
              valueFrom:
                secretKeyRef:
                  name: {{ include "lens.runtimeSecretName" . | quote }}
                  key: {{ .Values.agents.runtimeSecret.key | quote }}
          ports:
            - name: http
              containerPort: 4320
          startupProbe:
            httpGet: {path: /health, port: http}
            periodSeconds: 5
            failureThreshold: 60
          readinessProbe:
            httpGet: {path: /health, port: http}
          livenessProbe:
            httpGet: {path: /health, port: http}
          resources:
            {{- toYaml .Values.agents.resources | nindent 12 }}
          volumeMounts:
            - name: state
              mountPath: /var/lib/lens-runtime
            - name: tmp
              mountPath: /tmp
      volumes:
        - name: state
          persistentVolumeClaim:
            claimName: {{ $name }}-native
        - name: tmp
          emptyDir:
            medium: Memory
            sizeLimit: 512Mi
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
---
apiVersion: v1
kind: Service
metadata:
  name: {{ $name }}-agents
spec:
  selector:
    app.kubernetes.io/instance: {{ $instance }}
    app.kubernetes.io/component: lens-agents
  ports:
    - name: http
      port: 4320
      targetPort: http
---
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ $name }}-agents
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/instance: {{ $instance }}
      app.kubernetes.io/component: lens-agents
  policyTypes: [Ingress, Egress]
  ingress:
    - from:
        - podSelector:
            matchLabels:
              {{- include "lens.selectorLabels" . | nindent 14 }}
      ports: [{protocol: TCP, port: 4320}]
  egress:
    - to:
        - podSelector:
            matchLabels:
              {{- include "lens.selectorLabels" . | nindent 14 }}
      ports: [{protocol: TCP, port: 4318}]
    - to:
        - namespaceSelector:
            matchLabels:
              kubernetes.io/metadata.name: kube-system
      ports: [{protocol: UDP, port: 53}, {protocol: TCP, port: 53}]
    - to:
        - ipBlock:
            cidr: 0.0.0.0/0
            except: [10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 127.0.0.0/8]
{{- end }}
{{- end -}}
