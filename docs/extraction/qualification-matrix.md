# Candidate qualification

This matrix identifies the extraction candidates under test. They are development artifacts, not an official Lens release. The [implementation ledger](implementation-state.json) records completed checks, open work and release prerequisites

## Sources and contracts

| Component | Qualified source or contract | Delivery |
| --- | --- | --- |
| Extraction baseline | LiteLLM `0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9` | Original behavior and source history |
| Lens service and standalone UI | Lens `70c2d61e36503116241dcc2fbf5f51ed69cbf430`, version `0.1.0-dev.0` | Source on Lens main; immutable local image |
| Shared UI adopted by LiteLLM | Lens `af29e72a67893e9eb1e682ad0aee656647c63d20`, package `@litellm/lens-ui@0.1.0-dev.0` | Vendored tarball with npm integrity and source provenance |
| LiteLLM host and remote adapter | LiteLLM `d171e208a18d3f3559e3a338f7769387e01749e5`; production backend unchanged from `ba4cc3de`, shared UI updated in `d83eab03` | [PR 45529](https://github.com/BerriAI/litellm/pull/45529) |
| Public Lens API and eval SDK | Contract `1`; SDK source version `0.1.0a3` | Version checked at the API boundary; SDK has its own release lifecycle |
| Internal runtime contract | Protocol `7` | Internal Lens execution; retired external worker enrollment is not supported |

The shared UI source is unchanged between `af29e72a` and `70c2d61e`. The `af29e72a` change fixes metadata-condition insertion, with [regression coverage](evidence/metadata-filter-regression.json) and [final-image browser checks in both shells](evidence/final-image-ui-controls.json). The gateway bundles the package at build time and serves it with its own dashboard. It does not fetch executable UI code from a remote server at runtime. Adopting a new embedded UI requires updating the reviewed package and lockfile; a compatible Lens backend can be upgraded independently

The native sidebar, Agents and Evals additions came from concurrent shared UI work. The original horizontal navigation shell is not pixel-identical to the current shell. [Final-image paired browser evidence](evidence/final70-paired-ui.json) records preserved feature panels and the deliberate navigation differences

## Deployment boundary

| Configuration | Supported boundary | Evidence |
| --- | --- | --- |
| Standalone Compose | Lens and one ClickHouse server with in-process Keeper, persistent volume, localhost binding by default | [Fresh installation, restart, backup and restore](evidence/successor-container-qualification.json) |
| External ClickHouse | One stable endpoint to one server with KeeperMap and required permissions; multiple Lens instances may share it | [Capability and consistency checks](evidence/external-clickhouse-boundary.json) |
| Calculations | Native Linux with Landlock ABI 3 and seccomp, including the supported Docker Linux VM | Ten executed confinement and resource-limit cases in the container receipt |
| Embedded LiteLLM | Gateway login and authorized scopes delegated to Lens; shared UI with gateway-specific log drawer and routing | [Native adapter qualification](https://github.com/BerriAI/litellm/blob/ba4cc3ded3aa805eac1808a8ae3eff7271c425db/docs/evidence/lens-native-isolation.json) |
| Kubernetes | Standalone Lens chart and both LiteLLM chart families, with pinned compatible Lens artifacts | [Final-image Argo upgrade and rollback](evidence/helm/argo-successor-final-image.json); [both gateway chart families passed final-image install, independent upgrades, rollback, restart, external and disabled modes](evidence/helm/gateway-candidate-final.json) |

The local immutable Lens image is `sha256:6592e532802c79b08bb45a26e7a332d1711b9f862984e7c7d45702fad6d060ef` on Linux arm64. The [image switch receipt](evidence/final-image-switch.json) identifies its source and binary hashes. Final native amd64 candidate image is `sha256:66c6a367008c44c2ffc374448ffe58a683e815bf2c679e40f2d61d5c3b6c203c`. [Native amd64 and arm64 CI](evidence/final-native-ci.json) passed on `a354c657`, whose production and test sources are byte-identical to `70c2d61e`: 3,975 Rust tests, 93 UI tests, and both container recovery and confinement checks. Local qualification is not evidence that a signed multi-platform release has been published

Real paid investigations, eval judging and Signals used configured hosted provider endpoints. Tests cover native provider request and error contracts, but no live call to an official provider endpoint is claimed without that provider's credential. This distinction does not introduce a dependency on a locally deployed LiteLLM gateway

## Scope of carried evidence

The service binary in the final arm64 image is byte-identical to the previously tested `105e7b09` image. The only production changes since that source are the UI metadata-condition guard and importer preflight validation. Receipts retain the source actually executed; they are not relabeled as final-image runs. The [reconciliation proposal](evidence/acceptance-reconciliation-proposal.json) maps unchanged domain and storage code to its tests and records their limits

The [populated transfer](evidence/populated-migration.json), [both-shell restored data](evidence/populated-cutover-ui.json), [gateway disconnection](evidence/gateway-disconnection.json), [paired trace benchmark](evidence/service-benchmark-paired.json) and [fresh developer setup](evidence/fresh-developer.json), including a separate [actual Rust edit, rebuild and restoration](evidence/fresh-developer-rust-source.json), passed on their recorded sources. The final image separately passed [pending Signals process recovery](evidence/signal-pending-restart.json), browser controls, [four version-recorded framework runs](evidence/framework-versions-final70.json) and the Argo rehearsal

## Removal and publication gates

The clean cut removes old Lens API aliases, external worker enrollment and duplicated feature implementations. It retains the ordinary gateway inference path, authorization, billing, the independent ClickHouse spend callback and unrelated data. Internal Lens transfer uses a documented stop, import, verify and restart sequence

Before calling the code ready to merge, resolve actionable review findings, reconcile relevant upstream changes and perform the separate hour-long removal audit. Official publication additionally requires publisher package ownership, signing credentials, release approval and a clean install from the published manifest. Those production actions are separate from publishing source to Lens main and preparing companion PRs
