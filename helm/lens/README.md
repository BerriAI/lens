# Deploy Lens with Helm

This chart installs the complete Lens API, UI and investigation runtime with ClickHouse and Keeper on one persistent volume. A LiteLLM gateway and PostgreSQL are not required

For optional help from your coding agent, use [Set it up for me](../../docs/setup-with-agent.md). Include that this deployment uses Helm; there are separate prompts for an existing gateway and external ClickHouse

For a source checkout, install the chart and forward its service:

```sh
helm upgrade --install lens ./helm/lens --namespace lens --create-namespace
kubectl --namespace lens port-forward service/lens 4318:4318
```

Open `http://localhost:4318/ui/`. Obtain the generated private login credential locally with `kubectl --namespace lens get secret lens-admin -o jsonpath='{.data.admin-token}' | base64 --decode`. Keep this credential private. Helm reuses generated credentials on upgrades, and uninstall retains them and the ClickHouse volume

The source chart defaults to the version in its `appVersion`. Before a public release exists, provide a built image through `image.repository` and `image.tag`. A published release is installed from `oci://ghcr.io/berriai/charts/lens` with an exact `--version`; verify its signed release manifest before adopting it

## Use existing ClickHouse

Create a Kubernetes Secret containing the authenticated ClickHouse URL and set `clickhouseSecret.name` and `clickhouseSecret.key`. This suppresses the bundled ClickHouse resources. The server must support KeeperMap, have Keeper configured, and allow Lens to initialize its tables. Use `clickhouseDatabase` to select the existing database

Payloads and Keeper metadata must be backed up and restored together. An external database is never deleted by this chart. Existing installations need the documented writer handoff and data migration before changing their runtime, not just a Helm image update

## Connect a gateway

Set `gateway.enabled=true` and `serviceTokenSecret.name` to the Secret also configured on the gateway. Lens uses that server credential for delegated identity and internal service requests. Its private admin credential remains separate

`publicUrl` is the Lens browser origin. `ingestionUrl` is the URL agents can reach and defaults to `publicUrl`. Enable `ingress` for an externally routed standalone UI and API. Supply analysis provider configuration through `extraEnv` or `extraEnvFrom`, using Secret references for credentials

The LiteLLM charts consume this exact chart as a dependency with `library=true` and render its named templates through a small adapter. `lensWorker.mode` selects `bundled`, `external` or `disabled`; the legacy `lensWorker.enabled` switch still works when mode is empty. Both gateway charts keep the existing service and deployment names. An external Lens deployment can preserve `/lens-ingest` with `lensWorker.externalServiceName` in the componentized gateway chart

## Select versions independently

`image.digest=sha256:<64 lowercase hex characters>` takes precedence over `image.tag`. Lens versions do not have to equal gateway versions. Changing the Lens image leaves gateway pod templates unchanged, and changing the gateway image leaves the Lens pod template unchanged

Use one release owner per resource. Moving a bundled deployment into a separate Helm release or Argo application requires retaining its storage and credentials, stopping old writers, transferring ownership, and then starting the new runtime. Do not enable two controllers for the same deployment

For a Helm ownership transfer, first upgrade the old gateway release with `lensWorker.retainResources=true` and stop its Lens writers. Upgrade that release to external mode so its saved manifest no longer owns Lens resources, while Helm retains them. Install the independent Lens release with `--take-ownership`, copying `fullnameOverride`, `nameOverride`, `component` and `clickhouse.nameOverride` from the existing resources. Set `instanceOverride` to the old release name to preserve immutable deployment and storage selectors. Reuse the existing secrets, restore the replica count and verify data before retiring the old ownership. The new release defaults `retainResources=false`, so later uninstalls follow normal Helm behavior while preserving credentials and PVCs

## Check the chart

```sh
helm lint helm/lens
helm unittest -f 'tests/*_tests.yaml' helm/lens
helm package helm/lens
```

The unit suites check standalone startup configuration, credential reuse, persistent storage, external storage, digest selection and gateway connection settings. Actual installation, upgrades and recovery need a Kubernetes rehearsal with the selected release image
