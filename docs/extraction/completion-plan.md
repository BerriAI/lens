# LiteLLM Lens extraction plan and completion requirements

**Done means a developer can install and use the complete Lens product without a LiteLLM gateway, existing users retain the Lens experience inside LiteLLM, and the supported release, deployment, upgrade and documentation paths work across the affected repositories.** Moving source files or passing unit tests alone does not meet that boundary

This is a plan for a behavior-preserving extraction. Copy existing code and tests wherever possible. Confine necessary changes to application startup, authentication, persistence adapters, model access, packaging and integration. Preserve prompts, investigation logic, normalization, public data shapes, defaults and UI behavior. The authorized persistence redesign moves all Lens state into the ClickHouse stack while preserving the current transactional behavior. Other cleanup or redesign remains separate from the extraction

**The backend target is now Rust.** The user's subsequent instruction supersedes the earlier decision to retain a Python API. Follow the crate boundaries and directory layout in [the Rust migration specification](../rust-migration.md): Rust backend in `src/worker`, shared TypeScript UI and host in `src/ui`, and the Python eval SDK in `src/sdk`. The SDK and sandboxed analysis interpreter remain the documented Python exceptions. The directory move landed on main at `514e1b9d`; new work must use those paths

Port behavior and replay existing fixtures before replacing each Python module. Keep the current working application until its Rust replacement is integrated and qualified; an unused Rust implementation alone does not complete a module migration. The final Rust service owns HTTP, ingestion and investigation execution. Preserve supported worker, release and compatibility contracts during the transition

This change of implementation language does not reduce the completion boundary below. In particular, the embedded UI, connected mode, project-releaser, ops, both charts, existing-data migration and recovery remain required even though the Rust migration specification describes a narrower standalone scope. Preserve migration-source material until the populated import and recovery requirements pass. Publish collaboration changes directly to main as the user instructed; production release and cutover remain a separate approval boundary

**Lens plus ClickHouse is the complete standalone deployment.** This requirement supersedes the earlier PostgreSQL recommendation. Lens owns its settings, credentials, sessions, investigations, scheduling, budgets, checkpoints, datasets and signals in addition to its traces and feedback. None may require PostgreSQL, Redis, SQLite, a gateway database or another separately operated state service. The setup bundle must configure the entire supported ClickHouse stack automatically

The initial implementation candidate uses ordinary ClickHouse tables for record payloads and ClickHouse Keeper for atomic publication of small record pointers through KeeperMap. The default single-node installation enables Keeper inside the ClickHouse server container. Production replication may use the supported ClickHouse coordination topology, but no hidden application database is allowed. External ClickHouse installations must pass an explicit capability preflight; a URL accepting analytical queries alone does not prove the required coordination features are available. This design is subject to the correctness and capacity gates below

| Requirement | Pass condition | Evidence |
| --- | --- | --- |
| C1. ClickHouse is sufficient | The complete standalone Lens flow runs with only Lens services and the documented ClickHouse stack; no PostgreSQL connection or alternate persistence service exists in its runtime dependency graph | Fresh install, process/configuration inventory, restart and complete standalone workflow |
| C2. Preserve atomic decisions | Concurrent claims have one winner, dataset revision conflicts preserve the winning revision, stale progress cannot overwrite current work, and budget reservations/settlements preserve the existing limits | Existing behavior cases adapted to real ClickHouse, with multiple independent API clients and workers |
| C3. Publish related state together | Progress and its review checkpoint, state changes and archived runs, and other coupled changes are visible as a committed unit or remain unapplied | Failure before publication, contention during publication, lost response and storage/process restart exercises |
| C4. Recover and maintain storage | A crash cannot publish a missing payload. Old or losing versions can be reclaimed without deleting live records or making a stale write eligible again. Backup and restore retain both payloads and coordination metadata | Fault-injection, garbage-collection races, bounded-storage workload and actual restore rehearsal |
| C5. Support the ClickHouse deployment boundary | Bundled and explicitly supported external ClickHouse deployments initialize, migrate and upgrade automatically. Supported multi-replica API and ClickHouse modes retain consistency; missing capabilities fail preflight with actionable diagnostics | Pinned version/configuration matrix, fresh setup and upgrade tests, cross-client/cross-replica checks |
| C6. Finish the existing PostgreSQL dependency | Import existing Lens-owned PostgreSQL data without changing IDs, scopes, credentials or behavior; hand off writers once and remove the Lens runtime dependency afterward | Populated migration, standalone and embedded parity after PostgreSQL access is removed, rollback/restore rehearsal |

These six requirements add to every existing requirement below. They do not weaken parity, capacity, the embedded UI, project-releaser, ops, migration or recovery requirements. A single API process or an in-memory mutex is insufficient evidence for the concurrency boundary

This completion contract takes precedence over implementation suggestions in the earlier research proposal. The initial embedding approach is a versioned shared UI package mounted in the existing LiteLLM page. Preserve the existing spend-log drawer. Independently updating the embedded UI through an iframe is outside the first extraction

**The embedded UI is part of Lens.** Its Lens screens, components, behavior and tests live in the Lens repo and are developed and released there. Standalone and embedded are two ways of hosting the same Lens UI. LiteLLM owns its surrounding dashboard and gateway-specific integration adapters. A Lens screen change is implemented once in Lens; adopting its new package version in LiteLLM is a delivery step, not a second UI implementation.

All requirements below are planned and remain unverified until implementation. Each requirement needs evidence attached to the exact release candidate. A failed, skipped or blocked requirement remains incomplete; documenting it does not turn it into a pass

**There are three supported ways to use the same Lens product.**

| Mode | User experience | Dependency boundary |
| --- | --- | --- |
| Standalone | Open Lens's own URL, sign in, record traces, investigate, manage findings, feedback, datasets and signals | No gateway process, gateway database, gateway user account or gateway model key is required. Ordinary versioned SDK/library dependencies are allowed |
| Embedded | Sign in to LiteLLM normally and open the existing Lens page | Existing Lens navigation, permissions, interactions and gateway integrations remain available through the shared UI and a narrow server adapter |
| Connected | Open standalone Lens while optionally using LiteLLM for analysis models or gateway request/cost information | Gateway-dependent functions require the configured connection. Lens-owned tracing, data and direct-provider analysis remain independent |

Standalone and embedded views configured for the same Lens deployment must read and update the same authorized records. Separate installations need no automatic data synchronization. Gateway administration itself, such as managing all LiteLLM virtual keys or models, is not being rebuilt as a standalone Lens feature

The intentional new experiences are the standalone shell/login, standalone analysis connections, and independently selected Lens versions. Existing embedded flows must retain parity. Gateway-specific billing records require a gateway source; missing billing data must retain its correct unknown/unmatched meaning

**Base the extraction on the latest remote main commit.** Immediately before implementation, fetch BerriAI/litellm main and use that exact commit as the extraction source, branch/worktree base and parity baseline. Do not start from a stale local checkout or the earlier research snapshot. Fetch the latest main of each companion repo before making its integration changes, and record all full commit SHAs in the implementation ledger. Track relevant changes that land between that baseline and cutover, port them into the extraction and rerun affected acceptance cases.

The latest-main check on October 8, 2026 at 2:03 p.m. Pacific found LiteLLM at 0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9, three commits beyond the original research snapshot a7b04a8028cbf25ca85b740bd1c2d38dac4a3034. That revision has been fetched. This records the current observation; implementation must fetch again rather than treating this SHA as permanently latest. The source audit preserves the original research provenance and records the refresh separately.

| Requirement | Pass condition | Evidence |
| --- | --- | --- |
| B1. Complete baseline from latest main | Fetch the latest remote main at implementation start and record the exact source/base SHA. Inventory its screens, actions, API routes, accepted inputs, errors, permissions, defaults, models, storage and supported integrations. Include trace APIs and background tasks outside Lens directories; refresh companion repos before changing them | Recorded fetch time and remote main SHAs, extraction branch/worktree base, and a versioned feature/route inventory tied to that baseline |
| B2. Preserve behavior | Every baseline feature maps to its new owner and acceptance case. Necessary adapter changes are identifiable in the diff; product changes are kept separate | Source move map and reviewed behavior-change list |
| B3. Account for concurrent work | Every Lens change merged between baseline and cutover is included or shown to be unrelated. Outstanding PRs have an explicit destination | Final baseline-to-cutover reconciliation |
| B4. Bound the support promise | Record supported gateway/UI/Lens versions, deployment paths, integration versions and test workloads before qualifying the release | A checked-in compatibility and validation matrix |

**A developer must be able to get useful results from a fresh installation.**

| Requirement | User flow that must work | Evidence |
| --- | --- | --- |
| S1. Install without a gateway | On a clean supported machine, obtain the published Lens setup bundle and use one documented start command after installing its stated prerequisites. The command starts Lens and its complete ClickHouse stack. No local source compilation, sibling repo or PostgreSQL service is needed | Clean installation using published artifacts on supported amd64/arm64 container hosts, including macOS Docker and Linux |
| S2. First use without model credentials | Open the printed URL, complete local sign-in, explore the existing sample view, generate an ingestion key, send a test trace and open it without supplying a provider key | Browser recording plus ingestion/read responses; no paid model calls |
| S3. Connect an existing agent | Copy tracing settings into an agent already using its own provider. Record inputs, replies and tool activity without changing the agent's model endpoint or model login | A provider-direct framework example and the supported coding-agent flows |
| S4. Configure analysis separately | Choose a direct provider/model and budget in Lens, supply the credential through the supported configuration flow, and complete an investigation without a gateway | Persisted findings, evidence and analysis-cost records from a real provider call |
| S5. Start and stop safely | Restart services and rerun setup without replacing saved credentials, settings or data. Port conflicts, missing credentials and unavailable storage produce actionable diagnostics | Repeated setup/restart exercise with record and credential identity checks |
| S6. Operate through the API | A client can perform supported Lens operations with Lens authentication without first creating a LiteLLM account or virtual key | Public API workflow covering reads and representative writes |

**Every existing Lens feature must survive the move.** The baseline inventory extends this table if it finds another supported capability; this table is not permission to drop an unlisted existing feature

| Requirement | Required parity | Evidence |
| --- | --- | --- |
| F1. Ingestion and delivery | Existing supported OTLP trace/log formats, authentication, expiry/revocation, normalization, limits, error/retry behavior, receipt checks and /lens-ingest prefixes continue to work | Protocol fixtures plus real exporter tests, including Codex delivery receipts and Claude trace/log streams |
| F2. Browse and find activity | Agent lists, time windows, search, filters, sorting, live refresh, pagination and any exposed query/SQL capability return the same authorized activity | Baseline/new API comparisons and browser flows on empty, typical and large datasets |
| F3. Inspect complete traces | Tree, conversation, span content, attributes, errors, nested agents, resumed sessions, long content and partial-capture warnings preserve content, ordering and links | Representative trace fixtures, paging checks and paired UI captures |
| F4. Configure investigations | Create, edit, duplicate, preview and run investigations using current activity sources, expected behavior, checks, model selection, team/metadata filters, sampling and concurrency settings | Matching selections and saved configuration against the baseline |
| F5. Schedule and control runs | Monitoring, pause, run now, cancellation, progress, history and worker administration preserve semantics. Closing the browser does not stop execution; scheduled scans do not duplicate or overlap incorrectly | Lifecycle tests with restarts, multiple workers and a controlled clock |
| F6. Analyze evidence | Preserve prompts, tool access, calculation confinement, compaction, repair, evidence validation, grouping and reconciliation behavior | Deterministic model-response replay plus real-model investigations with valid source citations |
| F7. Read and act on findings | Needs-attention/pattern views, coverage, run selectors, accumulated findings, evidence navigation, resolve/dismiss/reopen and saved explanations behave as before | Browser flows and persisted state comparisons, including recurring evidence and finding aliases |
| F8. Reuse completed work | Review checkpoints, criteria/content fingerprints, partial results and unchanged reruns preserve reuse and cost behavior | Unchanged rerun makes no extra model calls when baseline reuse requires none; changed criteria/content trigger the appropriate work |
| F9. Datasets | Add recorded activity, edit expected responses and supported case fields, inspect historical revisions and export in the existing format | Revision/content/export comparisons with the baseline |
| F10. Trace feedback | Submit, read, update and delete supported end-user feedback; preserve ownership, aggregation and retention behavior | API/UI comparisons across authorized and unauthorized identities |
| F11. Signals | Configure the current evaluation-model connection and signals, classify eligible traces and display results with the same thresholds/retry behavior. Standalone uses a supported direct evaluation connection without a hidden gateway requirement | Classification replay and real-provider check; signal tasks survive application restart |
| F12. Costs and budgets | Preserve reservation, concurrency, settlement, cancellation and exhaustion behavior; context/output limits and errors remain meaningful. Distinguish actual gateway billing, SDK estimates and missing usage | Budget-contention and failure cases, plus recorded actual calls through direct and gateway analysis connections |

Standalone inference and gateway inference can have different configured prices and policies. Parity compares equivalent configured inputs and the accounting guarantees, not an invented promise that different providers or billing accounts charge the same amount

**Existing LiteLLM users must retain the same Lens experience.**

| Requirement | User flow or boundary that must hold | Evidence |
| --- | --- | --- |
| I1. Existing entrypoint | Sign in normally and open Lens through the existing navigation and legacy links without a second login | Browser checks for supported login methods, page routes and deployment prefixes |
| I2. Shared UI without interaction regressions | Existing layout, dialogs, tables, scrolling, focus/keyboard behavior, loading/error states and responsive behavior remain equivalent. Keep the spend-log drawer and back navigation | Same fixture-driven browser scenarios at baseline viewports; no unexplained visual or behavioral differences |
| I3. Durable navigation | Bookmark a trace, span, investigation, run, finding or dataset; refresh, reopen and use browser back/forward | Deep-link and route-state checks, including links issued before migration |
| I4. Correct identity | Preserve all baseline Lens permissions for admins, viewers, team/key/user scopes and feedback authors. Logout, expiry, revoked access and changed privileges cannot leave permanent Lens access | Allow/deny matrix and session lifecycle tests. Server authorization must not trust browser-provided role claims |
| I5. Gateway integration parity | Use existing model/key selection and analysis policies; view correlated request records, gateway costs and log detail. Retain IDs and tenant ownership used for joins | Existing connected workflows, including denied models and restricted keys |
| I6. Same deployment, same records | A permitted action from standalone Lens appears in embedded Lens, and vice versa, when both address the same service | Cross-view creation/update checks with identical IDs and scopes |
| I7. No frontend fork | Both shells consume the same versioned Lens UI source/package. LiteLLM owns only host integration and gateway-specific rendering adapters | Package dependency and build checks; no maintained duplicate Lens feature implementation |

Lens publishes the shared UI package as part of its release. The standalone application bundles that package, and LiteLLM bundles a pinned version of the same package. This is a delivery distinction; ownership of the embedded Lens UI remains in Lens. Upgrading the Lens API/runtime must keep the pinned UI working within the published compatibility range. Delivering a newer Lens UI package inside LiteLLM can require a LiteLLM frontend release.

**Existing data and failure behavior are part of parity.**

| Requirement | Pass condition | Evidence |
| --- | --- | --- |
| M1. Upgrade a populated installation | Traces, findings, aliases, run history, checkpoints, feedback, datasets/revisions, signal state and settings remain usable. Preserve IDs, ownership, key hashes and public ingestion addresses | Upgrade rehearsal with representative existing PostgreSQL/ClickHouse data, checksums/counts where applicable, and semantic/UI checks |
| M2. Preserve agent connections | Existing supported Lens tracing credentials and exporter URLs continue working through cutover without every developer reconfiguring their agent | Old clients export before and after migration with unchanged configuration |
| M3. Transfer ownership once | Schedules, signal loops and migrations have one active owner. Old and new services cannot both claim/write the same work incorrectly | Cutover exercise with active/queued work, drain or cancellation, and explicit writer handoff |
| M4. Convert analysis access correctly | Existing gateway key hashes are not treated as bearer credentials. The migration provisions or requests the required restricted outbound connection while preserving effective access/budget policy | Migration preflight and post-upgrade analysis under the intended key/account |
| M5. Recover without erasing data | Restart workers/services, recover expired leases, retain completed checkpoints and resume appropriate work without duplicate findings or incorrect spend | Fault-injection/recovery checks against the baseline guarantees |
| M6. Keep inference independent | A Lens or ClickHouse outage cannot block ordinary LiteLLM inference. Export queues remain bounded with visible failures/loss according to the existing delivery guarantees | Inference and export load test while Lens/storage is stopped |
| M7. Keep standalone independent | Remove or stop the optional gateway. Lens-owned ingestion continues past the old credential-cache lifetime; browsing, datasets and direct-provider analysis continue. Gateway-backed calls fail clearly | Disconnect exercise long enough to rule out cached gateway credentials hiding the dependency |
| M8. Safe rollback boundary | Operators can execute the documented rollback for the tested migration, or the documented restore/forward-recovery procedure when a database change prevents image-only rollback | Rehearsal with actual backups and recorded data-loss/downtime expectations; no claim of rollback based only on image tags |
| M9. Preserve capacity and isolation | Existing resource/query budgets and sandbox protections pass on the same workloads. No unexplained regression in latency, memory, ingestion throughput or dropped records | Paired runs on fixed hardware/data/settings; thresholds fixed before evaluating the candidate, using existing budgets and baseline variability |

Zero telemetry loss is not a new guarantee: preserve the baseline's documented bounded queues and retry/loss behavior, and demonstrate that the extraction has not worsened it. Likewise, preserve known baseline limitations explicitly rather than silently repairing or weakening behavior during the move

**project-releaser remains the approved publisher, with a focused Lens release path.** Today it detects Lens through paths in the LiteLLM source tree, builds the worker from the gateway commit and verifies its signature using the gateway release tag. The staging manifest has one product source/version. Removing the source directories without changing this contract could silently stop publishing Lens. The new contract must explicitly select either the supported legacy build path or the extracted Lens integration

The smallest useful ownership split is:

| Owner | Owns and publishes | Consumes |
| --- | --- | --- |
| litellm-lens | Lens API with standalone static UI, Rust runtime, shared React UI package, migrations, Compose/setup bundle, Lens Helm chart and compatibility metadata | Pinned library dependencies; optional gateway integration |
| project-releaser | Approved staging, verification, promotion, signing and release manifests for each product | An allowlisted source repository, full immutable commit, product version and release channel |
| litellm | Gateway, dashboard shell, Lens UI host adapter, delegated identity, request export and compatibility routes | A reviewed Lens lock/manifest and pinned UI package |
| litellm-ops | Environment selection, registry mirrors, deployment configuration, rollout validation and operational runbooks | Independently selected gateway and Lens releases |

Use one coordinated Lens version for its API, runtime, UI package and installation assets. Migrations run once as an explicit installation/upgrade step using the released Lens artifact. Retain exact API/runtime matching within Lens initially; independent releases require a bounded gateway-to-Lens and UI-to-API compatibility contract, not independent version negotiation for every internal component

The Lens release path in project-releaser should perform this sequence:

1. Resolve the approved Lens source commit and independent Lens version. Build the API/static UI, runtime and shared UI package in isolated builders; generate the chart and setup bundle from that same source.
2. Stage immutable image digests, the UI package tarball and its integrity hash, chart/setup checksums, migration identity and compatibility metadata. Preserve native amd64/arm64 runtime verification and the existing sandbox requirements.
3. Qualify these staged bytes in standalone and embedded installations. Use the exact staged UI package and images that will be promoted. Missing artifacts, an unsupported gateway/UI combination, invalid signatures or failed parity checks block the release.
4. Use the existing protected promotion process to publish those exact bytes. Do not rebuild after approval or resolve a mutable tag again. Sign and attest the Lens source/release identity, publish SBOMs as required by the current publisher, and publish the complete immutable manifest. Mark the release installable only when all required artifacts are available and verified.
5. Allow LiteLLM and ops to select that manifest explicitly. A Lens publication alone must not silently change every deployment or the Lens UI pinned by a gateway build.

A gateway release's reviewed Lens lock must identify the Lens release and source, API/runtime image digests, UI package version and integrity, chart version/checksum, release-manifest identity and supported integration contract. The gateway chart verifier must verify Lens's own identity and provenance. It must not require Lens to use the gateway's version or be re-signed as a gateway build

| Requirement | Pass condition | Evidence |
| --- | --- | --- |
| R1. Independent Lens publication | A Lens-only source change produces a complete Lens release without changing or rebuilding the gateway | Successful project-releaser staging/promotion with Lens source SHA, version and artifact manifest |
| R2. Build once, promote exactly | Approved artifacts and published artifacts have identical digests/checksums; required artifacts are complete | Staging-to-publication comparison, including UI package and setup/chart assets |
| R3. Preserve publisher trust | Only authorized release workflows write official Lens packages. Development publications remain separate; arbitrary source-owned scripts do not run privileged on the publisher host | Reviewed permissions/workflow diff, signature/provenance checks and negative unauthorized/mismatched-source cases |
| R4. Consume Lens from LiteLLM | A gateway release builds its embedded UI with the pinned package and packages/verifies the selected Lens chart/artifacts without compiling Lens from the gateway tree | Gateway release build using only its declared Lens artifacts, followed by an embedded functional check |
| R5. Separate version identities | Supported gateway and Lens versions can differ. The Lens API/runtime pair still enforces its own matching release and protocol | Valid unequal gateway/Lens release test and rejected API/runtime mismatch |
| R6. Test the compatibility boundary | The published matrix identifies tested gateway host adapters, embedded UI packages and Lens APIs. Incompatible combinations fail preflight before destructive changes | Current supported combinations, older pinned UI against newer compatible API, and a deliberately incompatible candidate |
| R7. Keep existing release paths | Standard/Rust gateway streams, supported older source branches, chart-only recovery and current private-source exclusion policy keep their intended behavior | Publisher regression fixtures plus relevant staging/recovery runs; no silent Lens omission on extracted branches |
| R8. Recover a partial publication | Failed pushes, interrupted promotion and retries cannot expose an apparently complete but inconsistent release or replace immutable bytes | Failure/retry rehearsal; idempotent resume or explicit failed-release state |
| R9. Install what was published | A new user can download the public supported release bundle and install it without unreleased local files, privileged registry access or an assumed matching gateway tag | Clean-machine install from the final published manifest and checksums |
| R10. Keep upgrades deliberate | Lens-only and gateway-only release selections remain independent, bounded by tested compatibility. Rolling labels do not bypass digest pins or compatibility checks | Independent version-bump diffs and rejected incompatible selections |

Semantic version labels alone do not prove compatibility. The initial matrix can be small: the first integration-ready LiteLLM release, the first standalone Lens release, and then a compatible Lens upgrade tested against the already embedded UI. Support expands only with evidence. Older gateway releases that require exact worker/gateway equality need the integration migration before independent Lens upgrades

For initial qualification, the independent-upgrade rehearsal may use successive immutable release candidates; it does not require inventing an unrelated public bug-fix release. Record precisely which combinations were exercised, and rerun the relevant checks when selecting the final published artifacts

**For example, releasing and upgrading Lens would work like this.** G1/G2 and L1/L2/L3 below are illustrative versions, not existing releases

| Action | What is built or deployed | What the user experiences |
| --- | --- | --- |
| Publish Lens L1 from Lens commit A | project-releaser publishes L1 API/static UI, runtime, shared UI package, chart and setup manifest | A new user starts the complete Lens product without LiteLLM |
| Publish integration-ready LiteLLM G1 | G1 embeds the L1 UI package and pins the L1 deployment manifest | Existing users open Lens inside LiteLLM with their existing session, routes and spend-log drawer |
| Publish a compatible Lens L2 fix from Lens commit B | project-releaser publishes Lens artifacts; no gateway source change or build is needed | Standalone users can install or upgrade directly to L2 |
| Upgrade an embedded installation to L2 | ops changes Lens API/runtime and standalone UI pins together, leaving G1 and its embedded L1 UI unchanged | The fixed backend is used from both views; qualification proves the old embedded UI still works |
| Adopt new Lens UI features inside LiteLLM | G2 bumps the shared UI package and compatible manifest as appropriate | Embedded users receive those UI changes with G2; standalone users received them with the Lens release |
| Attempt an incompatible Lens L3 upgrade | Preflight rejects the combination until a compatible gateway adapter/UI and migration are available | Operators receive a clear compatibility error before cutover |

The first upgrade from the current monolith includes the integration-ready gateway and the data/ownership migration. After that transition, routine compatible Lens releases can be independent. Full embedded UI independence would require a different embedding design; it is deliberately deferred to preserve current interactions during this extraction

**litellm-ops and both gateway charts must support the new boundary.** The current dev workflow sparsely checks out old Lens paths and derives a common tag from LiteLLM main. Its rollout verifier expects the same tag for the gateway components and Lens worker. Both need explicit product selection. Keep the existing hourly automation, publication destinations and operational checks, but resolve Lens and gateway source/artifacts separately

Prefer consuming or mirroring an approved Lens artifact over rebuilding an official release in ops. Development builds may use the separate Lens development stream. Record the source and destination digests and provenance when mirroring to ECR/GAR; validate the exact deployed artifact rather than assuming an image tag proves its contents

| Requirement | Operator flow or boundary that must hold | Evidence |
| --- | --- | --- |
| O1. Independent deployment selection | Dev and managed release configurations select gateway and Lens versions separately. Scheduled dev builds source Lens from its own repo | Dev workflow run and environment configuration showing both source identities and expected digests |
| O2. Lens-only and gateway-only updates | A Lens upgrade does not rebuild or roll the gateway; a gateway-only update does not unexpectedly reset, downgrade or redeploy Lens | Both rollout exercises with pod/image and persisted-state comparisons |
| O3. Both gateway charts work | helm/litellm and helm/litellm-helm support bundled Lens and an explicitly configured external Lens service, plus Lens-disabled behavior | Render/install/upgrade checks for both charts and actual embedded trace/investigation flows |
| O4. Preserve existing deployments | Existing Lens values, ingress prefixes, secrets, service names, replica/resource settings, persistent volumes and external ClickHouse configuration are retained or explicitly migrated | Upgrade using representative current dev/prod values; no accidental replacement of storage or credentials |
| O5. Transfer chart/storage ownership safely | Moving resources under a Lens chart does not cause Helm/ArgoCD to delete/recreate persistent data or run two migration owners. Existing Lens-owned PostgreSQL records migrate to ClickHouse through a tested writer handoff; gateway-owned PostgreSQL data and credentials remain intact | Rehearsed resource adoption and populated data migration, including rollback/restore and single migration ownership |
| O6. Verify useful operation | Registry mirroring, ArgoCD reconciliation, service discovery and rollout-check RBAC cover the new API/runtime resources. Success requires a trace and investigation, not only ready pods | Rollout report showing selected digests, readiness and end-to-end records |
| O7. Preserve asynchronous gateway export | Request correlation and the auto-router forwarder use the correct destination and tracing credentials; backpressure remains bounded | Exported gateway request appears in Lens with the intended scope/cost correlation, plus Lens-outage inference check |

Bundled installation may reuse the existing ClickHouse deployment after capability checks and a safe ownership transition. PostgreSQL is a migration source for existing Lens installations, never a Lens runtime requirement. Preserve existing record IDs and public data shapes while translating the persistence representation. Do not remove the gateway's PostgreSQL deployment or its unrelated records

**Contributors, docs and external integrations must be able to use the result.**

| Requirement | User flow or boundary that must hold | Evidence |
| --- | --- | --- |
| D1. Develop from one repo | Clone Lens, install documented prerequisites and run one development command without a sibling checkout. API/UI reload work; frontend/API contributors can use a prebuilt runtime and Rust contributors can rebuild it | Fresh-clone walkthrough on supported development platforms, including containerized investigation tools on macOS |
| D2. Own the complete build | Lens has a clear repository layout for its API, shared UI, Rust runtime, installation assets, docs and tests. Lens-owned tests, prompts, schemas, migrations and both code-generation directions run there. Shared Rust/Python dependencies are pinned; gateway consumers use released/pinned interfaces | Clean dependency/build graph and test/code-generation runs without filesystem references to the old checkout |
| D3. Publish an accurate quickstart | The canonical install, first trace, first investigation, development and upgrade instructions match the published artifacts. Users see distinct tracing credentials and analysis credentials | A walkthrough performed verbatim by a clean environment; docs-site build and link checks |
| D4. Preserve docs entrypoints and imports | Existing Lens docs URLs and redirects work. The example-doc importer still records source provenance and generates valid pages | Docs import/build, existing URL checks and linked standalone/embedded guides |
| D5. Preserve supported agents/examples | Provider-direct and gateway-backed examples, Codex capture/upload/receipt and Claude trace/log normalization work with documented endpoints and keys | Real supported integration runs, receipt confirmation and expected conversations/tool spans visible in Lens |
| D6. Make current benchmarks usable | Preserve historical pinned experiments. Qualify the current Rust/service implementation with a current runner rather than claiming the old Python worker imports test it | A reproducible current benchmark run with source/artifact identity and comparison fixtures |

The example repo and docs importer do not need to move into Lens to satisfy this boundary. They need correct dependencies, endpoints and instructions. Likewise, a new CLI, SQLite backend, plugin system, runtime module federation, iframe shell and general-purpose multi-product release framework are outside this extraction

**Implement in phases with a stop gate at each boundary.** These phases define sequencing, not permission to stop with a partial product

| Phase | Work and repositories | Stop gate |
| --- | --- | --- |
| 0. Establish the latest-main acceptance baseline | Fetch remote main, base extraction on that exact LiteLLM commit and refresh affected companion repos; inventory all consumers and establish source/behavior fixtures, a version matrix and cutover assumptions | B1–B4; full baseline SHAs are recorded and every existing feature has an owner and acceptance case before code removal |
| 1. Copy code and expose narrow adapters | Create Lens source ownership; retain runtime, API logic, UI, prompts, tests and schema behavior; isolate auth/storage/model/startup dependencies | Clean independent build, D2, source move review and baseline replay against the extracted implementation |
| 2. Complete standalone Lens | Implement and qualify ClickHouse persistence; add the small standalone shell, Lens-owned credentials/model access and setup/dev commands; bring over all background tasks | C1–C5, S1–S6 using staged assets, F1–F12, M5/M7/M9 and D1; every feature works with the gateway and PostgreSQL absent |
| 3. Restore the embedded experience | Consume the shared UI package from LiteLLM; add identity, route and gateway-specific host adapters | I1–I7 and M6; baseline browser/API parity, existing routes and no second login |
| 4. Make independent releases real | Update project-releaser, manifests, both charts and ops; exercise staging, promotion, recovery and separate version selection | R1–R10 and chart/ops rehearsals O1–O7, using the final public artifacts where required |
| 5. Migrate and close every consumer | Rehearse populated upgrades/rollback and removal of the Lens PostgreSQL dependency, reconcile concurrent Lens changes, update docs/examples/benchmarks and run a managed canary | B3, C1–C6, M1–M9, O1–O7 and D1–D6; close all remaining requirements before selected environment cutover |

The target canary environment and release approvals are selected during implementation. This plan does not execute or authorize a production deployment. If required publisher permissions or deployment approvals are still pending, the status is “implementation ready; release/cutover pending,” not “done”

Push the complete extraction directly to main before optional cleanup so contributors can work from the same source. Keep its readiness status explicit: available source is not a runnable release. Qualify the runnable version through actual end-to-end user flows, including real ingestion, provider-backed investigations, persisted results, restarts and the embedded UI, before calling it ready

**Parity needs a concrete evidence ledger.** For every requirement ID, record the baseline commit, candidate commit, artifact digests, mode/configuration, reproducible command or browser steps, expected result, observed result and linked evidence. Mark it pass, fail, blocked or not run. Baseline defects remain documented baseline limitations; new regressions remain failures. Scope removal requires an explicit user decision rather than silently declaring a requirement inapplicable

Use copied tests and fixture-driven comparisons for deterministic behavior. Replay identical model responses to compare prompts, tool requests, state transitions, evidence references, reuse, reservations and accounting. Normalize only predeclared incidental timestamps/random IDs, preserving identity relationships and ownership; do not hide changed citations, missing evidence or different scope behind normalization. Real-model checks verify working analysis and valid evidence without requiring identical stochastic wording

For UI parity, run the same browser actions on the same data and supported viewports, compare captures, and verify navigation, focus, scrolling and saved state. For capacity, fix workloads and acceptable limits from existing budgets and baseline variability before evaluating the candidate. A green build, healthy pods or a screenshot of the landing page is insufficient evidence

**I would call this done only when all of the following are demonstrated with the released candidate:** a fresh user can start Lens with only its ClickHouse stack and use every Lens capability; an existing LiteLLM user retains the same embedded flows and authorized data; a populated installation migrates, upgrades and recovers safely with no Lens PostgreSQL runtime dependency; project-releaser can publish Lens independently and LiteLLM can consume it by a tested pin; ops can upgrade either product independently; both charts, docs and supported external integrations work; and the evidence ledger has no unresolved regression or incomplete required case

Source provenance is retained in the [source manifest](source-manifest.json), and qualification status is recorded in the [implementation ledger](implementation-state.json). The original local research directory is no longer available. The baseline [publisher setup](https://github.com/BerriAI/project-releaser/blob/5ed9ef148c612a9ce70b8a5c6649415bb41305ae/LENS_RELEASE_SETUP.md), [staging manifest construction](https://github.com/BerriAI/project-releaser/blob/5ed9ef148c612a9ce70b8a5c6649415bb41305ae/.github/scripts/src/litellm_release/steps_staging.py), [dev workflow](https://github.com/BerriAI/litellm-ops/blob/4d5c16a66103102c155839fe8a1fa0b9c44521da/.github/workflows/deploy-dev.yml) and [rollout verifier](https://github.com/BerriAI/litellm-ops/blob/4d5c16a66103102c155839fe8a1fa0b9c44521da/scripts/verify_dev_rollout.py) explain why the publisher and ops changes are part of the completion boundary
