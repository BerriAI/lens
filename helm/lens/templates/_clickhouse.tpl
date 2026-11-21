{{- define "lens.clickhouse" -}}
{{- if include "lens.bundledClickhouse" . }}
{{- $name := include "lens.clickhouseName" . }}
apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {{ $name }}
  {{- include "lens.annotations" (dict "root" .) | nindent 2 }}
spec:
  podSelector:
    matchLabels:
      app.kubernetes.io/instance: {{ .Values.instanceOverride | default .Release.Name }}
      app.kubernetes.io/component: lens-clickhouse
  policyTypes: [Ingress]
  ingress:
    - from:
        - podSelector:
            matchLabels:
              {{- include "lens.selectorLabels" . | nindent 14 }}
      ports:
        - protocol: TCP
          port: 8123
---
apiVersion: v1
kind: Service
metadata:
  name: {{ $name }}
  {{- include "lens.annotations" (dict "root" .) | nindent 2 }}
spec:
  clusterIP: None
  selector:
    app.kubernetes.io/instance: {{ .Values.instanceOverride | default .Release.Name }}
    app.kubernetes.io/component: lens-clickhouse
  ports:
    - name: http
      port: 8123
      targetPort: http
---
apiVersion: apps/v1
kind: StatefulSet
metadata:
  name: {{ $name }}
  {{- include "lens.annotations" (dict "root" .) | nindent 2 }}
spec:
  serviceName: {{ $name }}
  replicas: 1
  persistentVolumeClaimRetentionPolicy:
    whenDeleted: Retain
    whenScaled: Retain
  selector:
    matchLabels:
      app.kubernetes.io/instance: {{ .Values.instanceOverride | default .Release.Name }}
      app.kubernetes.io/component: lens-clickhouse
  template:
    metadata:
      labels:
        app.kubernetes.io/instance: {{ .Values.instanceOverride | default .Release.Name }}
        app.kubernetes.io/component: lens-clickhouse
      annotations:
        checksum/config: {{ include "lens.clickhouseConfig" . | sha256sum }}
    spec:
      automountServiceAccountToken: false
      {{- with .Values.imagePullSecrets }}
      imagePullSecrets:
        {{- toYaml . | nindent 8 }}
      {{- end }}
      securityContext:
        runAsNonRoot: true
        runAsUser: 101
        runAsGroup: 101
        fsGroup: 101
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: clickhouse
          image: {{ .Values.clickhouse.image | quote }}
          securityContext:
            allowPrivilegeEscalation: false
            capabilities:
              drop: [ALL]
          env:
            - name: CLICKHOUSE_USER
              value: {{ .Values.clickhouse.user | quote }}
            - name: CLICKHOUSE_DB
              value: {{ .Values.clickhouseDatabase | quote }}
            - name: CLICKHOUSE_PASSWORD
              valueFrom:
                secretKeyRef:
                  name: {{ $name }}
                  key: password
            - name: CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT
              value: "1"
          ports:
            - name: http
              containerPort: 8123
          startupProbe:
            httpGet:
              path: /ping
              port: http
            periodSeconds: 5
            timeoutSeconds: 3
            failureThreshold: 60
          readinessProbe:
            httpGet:
              path: /ping
              port: http
          livenessProbe:
            httpGet:
              path: /ping
              port: http
            timeoutSeconds: 3
          resources:
            {{- toYaml .Values.clickhouse.resources | nindent 12 }}
          volumeMounts:
            - name: data
              mountPath: /var/lib/clickhouse
            - name: config
              mountPath: /etc/clickhouse-server/config.d/lens-keeper.xml
              subPath: lens-keeper.xml
              readOnly: true
            - name: config
              mountPath: /etc/clickhouse-server/config.d/lens-logging.xml
              subPath: lens-logging.xml
              readOnly: true
      volumes:
        - name: config
          configMap:
            name: {{ $name }}
  volumeClaimTemplates:
    - metadata:
        name: data
      spec:
        accessModes: [ReadWriteOnce]
        {{- if ne .Values.clickhouse.storageClassName nil }}
        storageClassName: {{ .Values.clickhouse.storageClassName | quote }}
        {{- end }}
        resources:
          requests:
            storage: {{ .Values.clickhouse.storage | quote }}
---
apiVersion: v1
kind: ConfigMap
metadata:
  name: {{ $name }}
  {{- include "lens.annotations" (dict "root" .) | nindent 2 }}
data:
  {{- include "lens.clickhouseConfig" . | nindent 2 }}
{{- end }}
{{- end -}}

{{- define "lens.clickhouseConfig" -}}
lens-keeper.xml: |
  <clickhouse>
    <keeper_server>
      <tcp_port>9181</tcp_port>
      <server_id>1</server_id>
      <log_storage_path>/var/lib/clickhouse/coordination/log</log_storage_path>
      <snapshot_storage_path>/var/lib/clickhouse/coordination/snapshots</snapshot_storage_path>
      <coordination_settings>
        <operation_timeout_ms>10000</operation_timeout_ms>
        <quorum_reads>true</quorum_reads>
      </coordination_settings>
      <raft_configuration>
        <server><id>1</id><hostname>127.0.0.1</hostname><port>9234</port></server>
      </raft_configuration>
    </keeper_server>
    <zookeeper><node><host>127.0.0.1</host><port>9181</port></node></zookeeper>
    <keeper_map_path_prefix>/lens/keeper-map</keeper_map_path_prefix>
  </clickhouse>
lens-logging.xml: |
  <clickhouse>
      <logger>
          <level>information</level>
          <console>true</console>
          <console_log_level>warning</console_log_level>
      </logger>
      <text_log>
          <level>information</level>
          <ttl>event_date + INTERVAL 1 DAY DELETE</ttl>
      </text_log>
      <query_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></query_log>
      <trace_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></trace_log>
      <part_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></part_log>
      <metric_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></metric_log>
      <query_metric_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></query_metric_log>
      <query_views_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></query_views_log>
      <asynchronous_metric_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></asynchronous_metric_log>
      <asynchronous_insert_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></asynchronous_insert_log>
      <aggregated_zookeeper_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></aggregated_zookeeper_log>
      <zookeeper_connection_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></zookeeper_connection_log>
      <background_schedule_pool_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></background_schedule_pool_log>
      <error_log><ttl>event_date + INTERVAL 1 DAY DELETE</ttl></error_log>
      <processors_profile_log remove="1"/>
  </clickhouse>
{{- end -}}
