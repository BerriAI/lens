# Candidate qualification

This matrix identifies the extraction candidates under test. They are development artifacts, not an official Lens release. The [implementation ledger](implementation-state.json) records completed checks, open work and release prerequisites

## Sources and contracts

| Component | Qualified source or contract | Delivery |
| --- | --- | --- |
| Extraction baseline | LiteLLM `0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9` | Original behavior and source history |
| Lens service and standalone UI | Lens `5846697965ba06a0370f83220b12d217884df9da`, version `0.1.0-dev.0` | Source on Lens main; immutable local image |
| Shared UI adopted by LiteLLM | Lens `5846697965ba06a0370f83220b12d217884df9da`, package `@litellm/lens-ui@0.1.0-dev.0` | Vendored tarball with npm integrity and source provenance |
| LiteLLM host and remote adapter | LiteLLM `310295b24da5dd7dabe2e694eb29dafb1c406297`; current shared UI package and split qualification routing | [PR 45529](https://github.com/BerriAI/litellm/pull/45529) |
| Public Lens API and eval SDK | Contract `1`; SDK source version `0.1.0a3` | Version checked at the API boundary; SDK has its own release lifecycle |
| Internal runtime contract | Protocol `7` | Internal Lens execution; retired external worker enrollment is not supported |

The host package contains 389 files matching Lens `58466979`, with npm integrity and source provenance in the [current UI receipt](https://github.com/BerriAI/litellm/blob/310295b24da5dd7dabe2e694eb29dafb1c406297/docs/evidence/lens-latest-ui.json). The gateway bundles it at build time. Embedded Lens uses horizontal tabs below the Lens and agent heading; standalone Lens retains its sidebar. The [follow-up receipt](evidence/review-follow-up-qualification.json) records current browser, build and test evidence

Project Releaser currently requires an explicit Lens SHA, while the gateway uses its checked-in UI package. Automatically selecting both main revisions once per combined build, including the corresponding Lens UI, remains an implementation requirement in the [completion plan](completion-plan.md). Published artifacts must remain immutable, and Lens can release independently

## Deployment boundary

| Configuration | Supported boundary | Evidence |
| --- | --- | --- |
| Standalone Compose | Lens and one ClickHouse server with in-process Keeper, persistent volume, localhost binding by default | [Fresh installation, restart, backup and restore](evidence/successor-container-qualification.json) |
| External ClickHouse | One stable endpoint to one server with KeeperMap and required permissions; multiple Lens instances may share it | [Capability and consistency checks](evidence/external-clickhouse-boundary.json) |
| Calculations | Native Linux with Landlock ABI 3 and seccomp, including the supported Docker Linux VM | Ten executed confinement and resource-limit cases in the container receipt |
| Embedded LiteLLM | Gateway login and authorized scopes delegated to Lens; shared UI with gateway-specific log drawer and routing | [Native adapter qualification](https://github.com/BerriAI/litellm/blob/ba4cc3ded3aa805eac1808a8ae3eff7271c425db/docs/evidence/lens-native-isolation.json) |
| Kubernetes | Standalone Lens chart and both LiteLLM chart families, with pinned compatible Lens artifacts | [Final-image Argo upgrade and rollback](evidence/helm/argo-successor-final-image.json); [both gateway chart families passed final-image install, independent upgrades, rollback, restart, external and disabled modes](evidence/helm/gateway-candidate-final.json) |

Historical final-removal qualification used the local immutable Lens image `sha256:6592e532802c79b08bb45a26e7a332d1711b9f862984e7c7d45702fad6d060ef` on Linux arm64. The [image switch receipt](evidence/final-image-switch.json) identifies its source and binary hashes. Final native amd64 candidate image is `sha256:66c6a367008c44c2ffc374448ffe58a683e815bf2c679e40f2d61d5c3b6c203c`. [Native amd64 and arm64 CI](evidence/final-native-ci.json) passed on `a354c657`, whose production and test sources are byte-identical to `70c2d61e`: 3,975 Rust tests, 93 UI tests, and both container recovery and confinement checks. Local qualification is not evidence that a signed multi-platform release has been published

Current Lens `58466979` passed both native container CI lanes, including recovery, migration, installed SDK and calculation confinement. The Rust workspace lane failed during one ClickHouse fixture startup, before behavior assertions; its follow-up remains explicit in the [current receipt](evidence/review-follow-up-qualification.json). The latest local arm64 runtime, migration and SDK checks passed against image `sha256:7761c6e4cfa3882f80831120122ba42eb8e0053fbbd71005d1d4a70bd084ad75`

The [wider Kubernetes run](https://github.com/BerriAI/lens/actions/runs/37971245107) passed both chart families, including fresh bundled PostgreSQL in the monolith. Its images and harness retain their executed revisions in the follow-up receipt. Separately, the current `310295b2` split backend/gateway passed three paid inference and exact billing checks before, during and after a Lens outage, costing $0.000326

Real paid investigations, eval judging and Signals used configured hosted provider endpoints. Tests cover native provider request and error contracts, but no live call to an official provider endpoint is claimed without that provider's credential. This distinction does not introduce a dependency on a locally deployed LiteLLM gateway

## Scope of carried evidence

The service binary in the historical final-removal arm64 image is byte-identical to the previously tested `105e7b09` image. The only production changes since that source are the UI metadata-condition guard and importer preflight validation. Receipts retain the source actually executed; they are not relabeled as final-image runs. The [reconciliation proposal](evidence/acceptance-reconciliation-proposal.json) maps unchanged domain and storage code to its tests and records their limits

The [populated transfer](evidence/populated-migration.json), [both-shell restored data](evidence/populated-cutover-ui.json), [gateway disconnection](evidence/gateway-disconnection.json), [paired trace benchmark](evidence/service-benchmark-paired.json) and [fresh developer setup](evidence/fresh-developer.json), including a separate [actual Rust edit, rebuild and restoration](evidence/fresh-developer-rust-source.json), passed on their recorded sources. The final image separately passed [pending Signals process recovery](evidence/signal-pending-restart.json), browser controls, [four version-recorded framework runs](evidence/framework-versions-final70.json) and the Argo rehearsal

## Removal and publication gates

The clean cut removes old Lens API aliases, external worker enrollment and duplicated feature implementations. It retains the ordinary gateway inference path, authorization, billing, the independent ClickHouse spend callback and unrelated data. Internal Lens transfer uses a documented stop, import, verify and restart sequence

The [separate removal audit](evidence/final-removal-audit.json) ran from October 9, 2026, 11:59:21 UTC to 13:02:20 UTC, just under 63 minutes, after implementation qualification. Independent lanes checked removed callers, gateway authorization and spend storage, tenant boundaries, ClickHouse publication and concurrency, release provenance, chart ownership and current upstream changes. No unresolved removal regression was identified in the exercised boundaries. A cross-repository checkout credential issue and obsolete deployment instructions were fixed and verified during the audit

The audit does not establish absolute production safety. Lens source is now public. The unchanged tested examples are published in [examples PR3](https://github.com/BerriAI/litellm-lens-example/pull/3), and their original public source links resolve. The shared Rust dependency PR remains open, and incomplete or unavailable external review checks remain explicit in the ledger. Some standalone control routes rely on the documented reverse-proxy request-body cap; the tested storage boundary is one ClickHouse server with KeeperMap

Official publication requires publisher package ownership, signing credentials, release approval and a clean install from the published manifest. Managed deployment and the production data cutover also remain separate operator actions. Publishing source to Lens main and preparing companion PRs does not perform any of those actions
