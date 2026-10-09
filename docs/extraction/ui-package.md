# Shared UI packaging qualification

The implementation candidate is `b1635a5ee7c8b88a6f829503882794e64a2a8f06`

The UI now builds independently as `@litellm/lens-ui`, with `src/ui/app` providing the standalone shell. Production output is static; the target Python API will serve it at `/ui/`. No Node service is needed to serve the exported UI

The source baseline remains LiteLLM main commit `0721cffab2ecbde51cdef9aeca0ce1c16b3e0aa9`. The import closure contains 193 Lens production modules, including imported fixture modules. After excluding imports and comments and normalizing formatting with the TypeScript printer, 189 retain identical bodies. The four adapted modules support same-origin cookie requests and injected spend-log lookup/drawer components. See [source reuse](evidence/ui/source-reuse.json) and the expanded [source manifest](source-manifest.json)

Dependency versions were selected from the source dashboard's lockfile. Shared components, design tokens, Inter font, static logos and toast appearance were copied. Internal aliases became relative imports; the model-cost hook moved into the package's hooks directory; the broad gateway networking module was replaced with the trace calls and HTTP configuration that Lens uses

## Checks

`npm run typecheck:ui` passes. `npm run test:ui` passes 97 checks across five explicit files covering workspace navigation, trace details, spend-log lookup and return navigation, datasets, and cookie/bearer authentication headers. These include copied component tests with network doubles. The jsdom harness reports its existing canvas/layout limitations and deprecated environment API usage; it does not establish browser rendering parity

Three isolated source mutations are rejected by behavior assertions: sending an empty bearer header for a cookie session, removing the 30-minute lookup padding, and opening a spend log with a different request ID. These are targeted adapter checks, not a codebase-wide mutation score. See [mutation evidence](evidence/ui/adapter-mutations.json)

`npm run qualify:ui-package` packs the UI, installs that tarball into a separate temporary app, and builds its static export. The consumer receives no source-tree alias or sibling LiteLLM checkout. The qualified development tarball has 369 entries and integrity `sha512-lLUVoHkDOXzMCMlWlsEXB8k8fYDDJj0yUcBJwp9QDmoLhEqRV1KwgvraU0bGuP4PYcTJpIbuZMfkxB3qSxpnMQ==`. This is local artifact qualification; nothing was published to a package registry

Logs: [focused tests](evidence/ui/focused-tests.log), [TypeScript](evidence/ui/typecheck.log), [packed consumer build](evidence/ui/packed-build.log)

## Browser observations

The packed consumer's static output was served on localhost at `/ui/?demo=true`. Browser actions opened a trace, switched between Steps and Thread, opened findings and evidence, opened the datasets view, selected an investigation and its History tab, and reloaded that history deep link. The selected investigation and tab survived reload. Demo writes remain disabled, as in the baseline

The trace list was visually compared at 1440×900 and 768×900 against unchanged baseline Lens components in an equivalent isolated shell. The narrow header and table crowding is present in both. These are manual spot checks with the same demo fixture definitions; their relative timestamps and animation frames differ. They are not a pixel-diff qualification or a comparison inside the actual gateway dashboard

| View | Baseline | Extracted UI |
| --- | --- | --- |
| Desktop trace list | [1440px](evidence/ui/baseline-traces-1440.jpg) | [1440px](evidence/ui/candidate-traces-1440.jpg) |
| Narrow trace list | [768px](evidence/ui/baseline-traces-768.jpg) | [768px](evidence/ui/candidate-traces-768.jpg) |
| Trace detail | Pending paired comparison | [Preview](evidence/ui/standalone-trace-detail.jpg) |

## Remaining boundaries

The standalone live API, storage deployment qualification, model access, scheduler startup and full deployment remain incomplete. This UI work does not qualify real ingestion, provider-backed analysis, persistence, restart behavior or migrations

LiteLLM still needs to adopt the package and supply its actual native drawer, identity, API routing and permission adapters. Light/dark coverage, the full viewport matrix, keyboard/focus parity, all setup and live-write flows, shared routing prefixes, contract regeneration and published-artifact compatibility checks remain open. None of the completion plan's full-product acceptance requirements is closed by this UI preview
