# Candidate qualification

This matrix identifies the extraction candidates under test. They are development artifacts, not an official Lens release. The [implementation ledger](implementation-state.json) records completed checks, open work and release prerequisites

## Sources and contracts

| Component | Qualified source or contract | Delivery |
| --- | --- | --- |
| Extraction baseline | LiteLLM `0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9` | Original behavior and source history |
| Lens service and standalone UI | Lens `105e7b0923a17311e527aad4d480b57bab431000`, version `0.1.0-dev.0` | Source on Lens main; immutable local image |
| Shared UI adopted by LiteLLM | Lens `0b8e2c23f73588c4b3e40e850edd1bad1654c19c`, package `@litellm/lens-ui@0.1.0-dev.0` | Vendored tarball with npm integrity and source provenance |
| LiteLLM host and remote adapter | LiteLLM `ba4cc3ded3aa805eac1808a8ae3eff7271c425db` | [PR 45529](https://github.com/BerriAI/litellm/pull/45529) |
| Public Lens API and eval SDK | Contract `1`; SDK source version `0.1.0a3` | Version checked at the API boundary; SDK has its own release lifecycle |
| Internal runtime contract | Protocol `7` | Internal Lens execution; retired external worker enrollment is not supported |

The shared UI source is unchanged between `0b8e2c23` and `105e7b09`. The gateway bundles the package at build time and serves it with its own dashboard. It does not fetch executable UI code from a remote server at runtime. Adopting a new embedded UI requires updating the reviewed package and lockfile; a compatible Lens backend can be upgraded independently

The native sidebar, Agents and Evals additions came from concurrent shared UI work. The original horizontal navigation shell is not pixel-identical to the current shell. [Paired browser evidence](evidence/paired-current-ui.json) records preserved feature panels and the deliberate navigation differences

## Deployment boundary

| Configuration | Supported boundary | Evidence |
| --- | --- | --- |
| Standalone Compose | Lens and one ClickHouse server with in-process Keeper, persistent volume, localhost binding by default | [Fresh installation, restart, backup and restore](evidence/successor-container-qualification.json) |
| External ClickHouse | One stable endpoint to one server with KeeperMap and required permissions; multiple Lens instances may share it | [Capability and consistency checks](evidence/external-clickhouse-boundary.json) |
| Calculations | Native Linux with Landlock ABI 3 and seccomp, including the supported Docker Linux VM | Ten executed confinement and resource-limit cases in the container receipt |
| Embedded LiteLLM | Gateway login and authorized scopes delegated to Lens; shared UI with gateway-specific log drawer and routing | [Native adapter qualification](https://github.com/BerriAI/litellm/blob/ba4cc3ded3aa805eac1808a8ae3eff7271c425db/docs/evidence/lens-native-isolation.json) |
| Kubernetes | Standalone Lens chart and both LiteLLM chart families, with pinned compatible Lens artifacts | Final real installs and independent upgrade receipts remain tracked in the ledger |

The local immutable Lens image is `sha256:9a2eba30f01720fe66a6f45bcc3beba63caac2d3e44e59445bf1ccede7715563` on Linux arm64. Native amd64 and arm64 CI results are recorded separately. Local qualification is not evidence that a signed multi-platform release has been published

Real paid investigations, eval judging and Signals used configured hosted provider endpoints. Tests cover native provider request and error contracts, but no live call to an official provider endpoint is claimed without that provider's credential. This distinction does not introduce a dependency on a locally deployed LiteLLM gateway

## Removal and publication gates

The clean cut removes old Lens API aliases, external worker enrollment and duplicated feature implementations. It retains the ordinary gateway inference path, authorization, billing, the independent ClickHouse spend callback and unrelated data. Internal Lens transfer uses a documented stop, import, verify and restart sequence

Before calling the code ready to merge, complete the remaining image/chart and performance checks, resolve actionable review findings, reconcile relevant upstream changes and perform the separate hour-long removal audit. Official publication additionally requires publisher package ownership, signing credentials, release approval and a clean install from the published manifest. Those production actions are separate from publishing source to Lens main and preparing companion PRs
