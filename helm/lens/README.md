# Deploy Lens with Helm

This chart installs the complete Lens API, UI and investigation runtime with ClickHouse and Keeper on one persistent volume. A LiteLLM gateway and PostgreSQL are not required

For optional help from your coding agent, use [Set it up for me](../../docs/setup-with-agent.md). Include that this deployment uses Helm; there are separate prompts for an existing gateway and external ClickHouse

Until a Lens release is published, build from the repository root and push the image to a registry your cluster can read. Replace the example repository with your own, and configure the cluster's image pull credentials if that registry is private

```sh
LENS_IMAGE_REPOSITORY=registry.example.com/your-team/lens
LENS_IMAGE_TAG=$(git rev-parse HEAD)
docker build --build-arg LENS_VERSION="$LENS_IMAGE_TAG" -f deploy/runtime/Dockerfile \
  -t "$LENS_IMAGE_REPOSITORY:$LENS_IMAGE_TAG" .
docker push "$LENS_IMAGE_REPOSITORY:$LENS_IMAGE_TAG"
helm upgrade --install lens ./helm/lens --namespace lens --create-namespace \
  --set image.repository="$LENS_IMAGE_REPOSITORY" --set image.tag="$LENS_IMAGE_TAG"
kubectl --namespace lens port-forward service/lens 4318:4318
```

Open `http://localhost:4318/ui/`. Obtain the generated private login credential locally with `kubectl --namespace lens get secret lens-admin -o jsonpath='{.data.admin-token}' | base64 --decode`. Keep this credential private. Helm reuses generated credentials on upgrades, and uninstall retains them and the ClickHouse volume

The source chart defaults to the version in its `appVersion`. Before a public release exists, provide a built image through `image.repository` and `image.tag`. A published release is installed from `oci://ghcr.io/berriai/charts/lens` with an exact `--version`; verify its signed release manifest before adopting it

## Use existing ClickHouse

Skip this section when using the bundled database. Supported external storage is one ClickHouse server reached through a stable HTTP(S) endpoint. Several Lens replicas can share that endpoint and database. Every request must reach the same ClickHouse server; a load balancer that distributes requests across ClickHouse nodes, a distributed table deployment, and automatic failover to another ClickHouse server are unsupported

Lens stores record payloads and indexes in local `ReplacingMergeTree` tables. Keeper coordinates publication of their current revisions; sharing Keeper across ClickHouse nodes does not replicate those payloads. This boundary applies to standalone Lens and Lens connected to a LiteLLM gateway

Create a Kubernetes Secret containing the authenticated ClickHouse URL and set `clickhouseSecret.name` and `clickhouseSecret.key`. This suppresses the bundled ClickHouse resources. The server must support KeeperMap and have Keeper configured, including `keeper_map_path_prefix`; the bundled [Keeper configuration](../../deploy/clickhouse/keeper.xml) is the single-server example. Use `clickhouseDatabase` for a database dedicated to Lens. The database credential must allow Lens to create its database, tables and materialized views, read and insert rows, and alter tables for publication and cleanup. A dedicated user with `ALL ON <lens_database>.*` was qualified against ClickHouse 26.9.6.6

Startup initializes the schema and writes initial application state before opening the HTTP listener. Missing Keeper configuration or insufficient database permissions causes startup to fail. Check `kubectl --namespace lens logs deployment/lens` and the ClickHouse server logs, correct the configuration or grants, then repeat the installation check. Startup does not detect whether an endpoint distributes requests across servers; the operator must establish the topology above

The external-storage qualification on ClickHouse 26.9.6.6 observed these startup failures:

| Lens startup diagnostic | Configuration to check |
| --- | --- |
| `StateStorage(StateFailed { status: 400, code: Some(36) })` with an unconfigured KeeperMap server | Configure Keeper, the ClickHouse `zookeeper` connection and `keeper_map_path_prefix`, following the linked single-server example. Code 36 can also describe other invalid arguments, so use the ClickHouse logs to confirm the cause |
| `Storage(Storage(SchemaFailed(403)))` with a SELECT-only database user | Have the database administrator grant the Lens user the database creation, schema and write permissions listed above. Use a database dedicated to Lens |

Payloads and Keeper metadata must be backed up and restored together. An external database is never deleted by this chart. Existing installations need the documented writer handoff and data migration before changing their runtime, not just a Helm image update

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

Create `lens-connection` through your existing secret manager. Generate each value independently with `openssl rand -hex 32`. Its `gateway-secret` value must match the gateway's `LENS_GATEWAY_SECRET`; its `service-token` value must match `LITELLM_LENS_SERVICE_TOKEN`. Both values must contain at least 32 characters. In either LiteLLM chart, the corresponding settings are `lensWorker.gateway.secretName`, `lensWorker.gateway.secretKey` and `lensWorker.serviceTokenSecret`. Bundled Helm installations generate separate credentials when these references are unset; external mode requires both references

`publicUrl` is the Lens browser origin. `ingestionUrl` is the URL agents can reach and defaults to `publicUrl`. Enable `ingress` for an externally routed standalone UI and API. Supply analysis provider configuration through `extraEnv` or `extraEnvFrom`, using Secret references for credentials

The LiteLLM charts consume this exact chart as a dependency with `library=true` and render its named templates through a small adapter. `lensWorker.mode` selects `bundled`, `external` or `disabled`; the legacy `lensWorker.enabled` switch still works when mode is empty. Both gateway charts keep the existing service and deployment names. An external Lens deployment can preserve `/lens-ingest` with `lensWorker.externalServiceName` in either gateway chart

## Render with GitOps

Helm installations generate credentials once and reuse them through a live cluster lookup. Argo CD and other offline renderers cannot perform that lookup. For those deployments, provision secrets through your existing secret manager and set `adminTokenSecret.name`, `gateway.secretName` and `serviceTokenSecret.name` before rendering. The gateway references are needed only when `gateway.enabled=true`

The bundled database also generates a password. Use `clickhouseSecret.name` with your existing supported ClickHouse deployment for deterministic GitOps rendering. In the LiteLLM charts, prefix these settings with `lensWorker.`. Verify two consecutive renders produce identical Secret references before syncing; do not commit rendered secret values to Git

## Select versions independently

`image.digest=sha256:<64 lowercase hex characters>` takes precedence over `image.tag`. Official paired releases use the same Lens and gateway version. Existing deployments can select Lens separately when their API contracts remain compatible. Changing the Lens image leaves gateway pod templates unchanged, and changing the gateway image leaves the Lens pod template unchanged

Use one release owner per resource. Moving a bundled deployment into a separate Helm release or Argo application requires retaining its storage and credentials, stopping old writers, transferring ownership, and then starting the new runtime. Do not enable two controllers for the same deployment

For a Helm ownership transfer, first upgrade the old gateway release with `lensWorker.retainResources=true` and stop its Lens writers. Upgrade that release to external mode so its saved manifest no longer owns Lens resources, while Helm retains them. Install the independent Lens release with `--take-ownership`, copying `fullnameOverride`, `nameOverride`, `component` and `clickhouse.nameOverride` from the existing resources. Set `instanceOverride` to the old release name to preserve immutable deployment and storage selectors. Reuse the existing secrets, restore the replica count and verify data before retiring the old ownership. The new release defaults `retainResources=false`, so later uninstalls follow normal Helm behavior while preserving credentials and PVCs

## Check the chart

```sh
helm lint helm/lens
helm unittest -f 'tests/*_tests.yaml' helm/lens
helm package helm/lens
```

The unit suites check standalone startup configuration, credential reuse, persistent storage, external storage, digest selection and gateway connection settings. Actual installation, upgrades and recovery need a Kubernetes rehearsal with the selected release image
