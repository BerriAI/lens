# Deploy Lens with Helm

This chart installs the complete Lens API, UI and investigation runtime with ClickHouse and Keeper on one persistent volume. A LiteLLM gateway and PostgreSQL are not required

For optional help from your coding agent, use [Set it up for me](../../docs/setup-with-agent.md). Include that this deployment uses Helm; there are separate prompts for an existing gateway and external ClickHouse

Install the public Lens 0.4.0 chart. It pulls the release image by digest without a source checkout or build. Your cluster needs a default StorageClass, or set `clickhouse.storageClassName` explicitly. On EKS, install the [EBS CSI driver add-on](https://docs.aws.amazon.com/eks/latest/userguide/ebs-csi.html) and configure a StorageClass first

```sh
helm upgrade --install lens oci://ghcr.io/berriai/charts/lens \
  --version 0.4.0 --namespace lens --create-namespace --wait --timeout 10m
kubectl --namespace lens port-forward service/lens 4318:4318
```

Open `http://localhost:4318/ui/`. Obtain the generated private login credential locally with `kubectl --namespace lens get secret lens-admin -o jsonpath='{.data.admin-token}' | base64 --decode`. Keep this credential private. Helm reuses generated credentials on upgrades, and uninstall retains them and the ClickHouse volume

With bundled storage, the Lens pod stays in `Init` until ClickHouse answers. With your own ClickHouse, Lens restarts with `Error: Storage(Storage(Transport))` until it can connect. Persistent failures after that need the storage checks below. [Verify the signed release manifest](../../docs/releasing.md#verify-a-release) before adopting a release

The bundled ClickHouse deployment includes an ingress NetworkPolicy that permits TCP 8123 only from this release's Lens pods in the same namespace. Keeper and Raft ports are not exposed to other pods by that policy. Your cluster must use a CNI that enforces NetworkPolicy, and other policies selecting the ClickHouse pod must not grant broader access because Kubernetes combines their allowed traffic. External ClickHouse deployments need equivalent storage isolation managed by their operator

## Use existing ClickHouse

Skip this section when using the bundled database. Supported external storage is one ClickHouse server reached through a stable HTTP(S) endpoint. With the agent workspace disabled, several Lens replicas can share that endpoint and database. The agent workspace requires one Lens replica. Every request must reach the same ClickHouse server; a load balancer that distributes requests across ClickHouse nodes, a distributed table deployment, and automatic failover to another ClickHouse server are unsupported

Lens stores record payloads and indexes in local `ReplacingMergeTree` tables. Keeper coordinates publication of their current revisions; sharing Keeper across ClickHouse nodes does not replicate those payloads. This boundary applies to standalone Lens and Lens connected to a LiteLLM gateway

Create a Kubernetes Secret containing the authenticated ClickHouse URL and set `clickhouseSecret.name` and `clickhouseSecret.key`. This suppresses the bundled ClickHouse resources. The server must support KeeperMap and have Keeper configured, including `keeper_map_path_prefix`; the bundled [Keeper configuration](../../deploy/clickhouse/keeper.xml) is the single-server example. Use `clickhouseDatabase` for a database dedicated to Lens. The database credential must allow Lens to create its database, tables and materialized views, read and insert rows, and alter tables for publication and cleanup. A dedicated user with `ALL ON <lens_database>.*` was qualified against ClickHouse 26.9.6.6

Startup initializes the schema and writes initial application state before opening the HTTP listener. Missing Keeper configuration or insufficient database permissions causes startup to fail. Check `kubectl --namespace lens logs deployment/lens` and the ClickHouse server logs, correct the configuration or grants, then repeat the installation check. Startup does not detect whether an endpoint distributes requests across servers; the operator must establish the topology above

The external-storage qualification on ClickHouse 26.9.6.6 observed these startup failures:

| Lens startup diagnostic | Configuration to check |
| --- | --- |
| `StateStorage(StateFailed { status: 400, code: Some(36) })` with an unconfigured KeeperMap server | Configure Keeper, the ClickHouse `zookeeper` connection and `keeper_map_path_prefix`, following the linked single-server example. Code 36 can also describe other invalid arguments, so use the ClickHouse logs to confirm the cause |
| `Storage(Storage(SchemaFailed(403)))` with a SELECT-only database user | Have the database administrator grant the Lens user the database creation, schema and write permissions listed above. Use a database dedicated to Lens |

Payloads and Keeper metadata must be backed up and restored together. An external database is never deleted by this chart. Existing installations need the documented writer handoff and data migration before changing their runtime, not just a Helm image update

## Agent workspace

On this experiment branch, enable `agents.enabled` and supply the core and native images built from the same source revision. The chart creates separate Lens and native runtime Deployments with independent resources, a 5 GiB shared-state PVC and a 10 GiB native-state PVC. ClickHouse keeps its own storage

```yaml
replicaCount: 1
image:
  repository: registry.example/lens
  tag: candidate-revision
agents:
  enabled: true
  image:
    repository: registry.example/lens-agents
    tag: candidate-revision
extraEnvFrom:
  - secretRef:
      name: workspace-inference
```

Replace the two image references with your built candidates. Provision `workspace-inference` through your secret manager with `LITELLM_PROXY_API_BASE` and `LITELLM_PROXY_API_KEY`. That dedicated key determines the workspace's model and MCP access. To enable Slack, add `LENS_AGENT_SLACK_BOT_TOKEN` and `LENS_AGENT_SLACK_APP_TOKEN` to the same Secret, following the [Slack setup](../../deploy/lens/README.md#connect-slack)

Save the non-secret settings above as `agent-values.yaml`, then install from this checkout:

```sh
helm upgrade --install lens ./helm/lens \
  --namespace lens --create-namespace --values agent-values.yaml --wait
kubectl --namespace lens get deployments,pvc
kubectl --namespace lens port-forward service/lens 4318:4318
```

Open `http://localhost:4318/ui/`, sign in using the generated Lens admin credential described above, and send a message from Home. The response must complete before considering native inference connected. Create a bot and verify that its first run completes; pause it when finished with the check

Lens and the native runtime each require one replica in this version. The chart rejects a larger Lens replica count while agents are enabled and uses `Recreate` updates to avoid concurrent state owners. Each service can have its resources adjusted independently. Additional gateway replicas are a separate gateway-chart setting; horizontally scaling the Lens workspace needs a shared database and distributed bot ownership first

The native runtime receives only its internal runtime credential. It has no gateway, storage or Kubernetes API credentials. It runs agents under separate Linux users and needs the capabilities declared in the chart for user and file ownership changes. Its NetworkPolicy permits Lens, cluster DNS and public outbound tools, while excluding private and link-local destinations. Use a CNI that enforces NetworkPolicy, and verify those boundaries in the target cluster

Set `agents.storageClassName` and `clickhouse.storageClassName` for your cluster. For EKS Auto Mode, create an encrypted `gp3` StorageClass using `ebs.csi.eks.amazonaws.com`, `WaitForFirstConsumer` and `Retain`; EKS does not create it automatically. Enable its network-policy controller before running bots, as described in the [AWS guide](https://docs.aws.amazon.com/eks/latest/userguide/auto-net-pol.html)

Helm generates and retains the runtime credential across upgrades. For offline GitOps rendering, supply `agents.runtimeSecret.name` and `agents.runtimeSecret.key`. Agent PVCs are retained on uninstall. Before a stateful upgrade, stop Lens and the native runtime and snapshot their two volumes, ClickHouse data and Keeper metadata, and credentials together. Restarting retains conversations and bot memory; it does not replay interrupted turns

## Connect a gateway

Set `gateway.enabled=true` and configure two separate values shared with the gateway. The service token authenticates internal data requests; the signing secret authenticates delegated user identities. The Lens admin login credential remains separate

Reference the two keys from your secret manager:

```yaml
gateway:
  enabled: true
  secretName: lens-connection
  secretKey: gateway-secret
serviceTokenSecret:
  name: lens-connection
  key: service-token
```

Create `lens-connection` through your secret manager. Generate each value independently with `openssl rand -hex 32`. Its `gateway-secret` value must match the gateway's `LENS_GATEWAY_SECRET`; its `service-token` value must match `LITELLM_LENS_SERVICE_TOKEN`. Both values must contain at least 32 characters. If the services use different namespaces, create the Secret with identical values in both namespaces

With `gateway.enabled`, the standalone sign-in page offers Log in with LiteLLM once Lens knows the gateway's browser address: set `LENS_GATEWAY_PUBLIC_URL` through `extraEnv` (it falls back to `LENS_GATEWAY_URL`, then `LENS_GATEWAY_API_BASE`), and make the gateway's `lens.publicUrl` the Lens browser origin, since the gateway sends the browser back there after sign-in

`publicUrl` is the Lens browser origin, and Lens rejects sign-in and saves from any other origin. Left empty, it is the `ingress.host` address when `ingress` is enabled (`https` when `ingress.tls` is set, otherwise `http`) and `http://localhost:4318` without ingress. Set it when browsers use a different scheme or port, for example TLS terminated at a load balancer. The install fails when `publicUrl` names a different host than `ingress.host`. `ingestionUrl` is the URL agents can reach and defaults to `publicUrl`. Enable `ingress` for an externally routed standalone UI and API, or set `service.type` to `NodePort` or `LoadBalancer` (with `service.nodePort` for a fixed port) and set `publicUrl` to the address browsers open. Supply analysis provider configuration through `extraEnv` or `extraEnvFrom`, using Secret references for credentials

The gateway chart at `oci://ghcr.io/berriai/litellm/chart/litellm` does not deploy Lens or PostgreSQL. For chart version `1.111.0-rc.2`, set `lens.enabled: true`, `lens.url`, `lens.publicUrl`, `lens.gatewaySecret.name` and `lens.serviceTokenSecret.name`. The Secret keys default to `gateway-secret` and `service-token`. Removed `lensWorker.*` values are not supported and enabling them fails the render

For Lens in namespace `lens`, the gateway's `lens.url` can be `http://lens.lens.svc.cluster.local:4318`. With port-forward only, use `http://localhost:4318` for Lens `publicUrl` and gateway `lens.publicUrl`. This address works only for browsers and agents on the computer with the open tunnel. Use reachable HTTPS addresses for team access. The [complete Kubernetes example](https://docs.litellm.ai/docs/proxy/lens/deployment/kubernetes#existing-deployment) includes both namespaces, gateway database and master key setup

## Render with GitOps

Helm installations generate credentials once and reuse them through a live cluster lookup. Argo CD and other offline renderers cannot perform that lookup. For those deployments, provision secrets through your existing secret manager and set `adminTokenSecret.name`, `gateway.secretName` and `serviceTokenSecret.name` before rendering. The gateway references are needed only when `gateway.enabled=true`

The bundled database also generates a password. Use `clickhouseSecret.name` with your existing supported ClickHouse deployment for deterministic GitOps rendering. These settings belong to the separate Lens release, not the gateway chart. Verify two consecutive renders produce identical Secret references before syncing; do not commit rendered secret values to Git

## Select versions independently

`image.digest=sha256:<64 lowercase hex characters>` takes precedence over `image.tag`. Select Lens and gateway versions independently when their API contracts are compatible. Changing the Lens image leaves gateway pod templates unchanged, and changing the gateway image leaves the Lens pod template unchanged

Use one release owner per resource. Moving a bundled deployment into a separate Helm release or Argo application requires retaining its storage and credentials, stopping old writers, transferring ownership, and then starting the new runtime. Do not enable two controllers for the same deployment

For a Helm ownership transfer from a legacy gateway chart that bundled Lens, first use that old chart's `lensWorker.retainResources=true` and stop its Lens writers. Move that old release to external mode so its saved manifest no longer owns Lens resources, while Helm retains them. Do this before adopting the current gateway chart, which rejects `lensWorker.*`. Install the independent Lens release with `--take-ownership`, copying `fullnameOverride`, `nameOverride`, `component` and `clickhouse.nameOverride` from the existing resources. Set `instanceOverride` to the old release name to preserve immutable deployment and storage selectors. Reuse the existing secrets, restore the replica count and verify data before retiring the old ownership. The new release defaults `retainResources=false`, so later uninstalls follow normal Helm behavior while preserving credentials and PVCs

## Check the chart

```sh
helm lint helm/lens
helm unittest -f 'tests/*_tests.yaml' helm/lens
helm package helm/lens
```

The unit suites check standalone startup configuration, credential reuse, persistent storage, external storage, digest selection and gateway connection settings. Actual installation, upgrades and recovery need a Kubernetes rehearsal with the selected release image
