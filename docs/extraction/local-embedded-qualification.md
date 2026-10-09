# Local LiteLLM embedding qualification

This runbook verifies that LiteLLM uses the packed Lens UI with its existing login, authorization, API client configuration and spend-log drawer. It is local qualification only. Shipping still requires an exact published package version and a committed lockfile with integrity; no runtime remote JavaScript loader or moving Git reference is involved

The delivery order now prioritizes standalone self-deployment as milestone 1. This local prototype and its evidence are retained for milestone 2; project-releaser, litellm-ops and the remaining plan follow in milestone 3

The integration worktree started from LiteLLM main `f48d837cd2`. The local adapter replaces the Lens page's `LensWorkspace` import with `EmbeddedLens`, which consumes `@litellm/lens-ui`. It delegates spend-log lookup and the drawer to LiteLLM, and supplies live API base, bearer token, header name, root prefix and error-handler bindings. Next transpiles the package, and the gateway stylesheet scans the installed package for Tailwind classes

See the [adapter diff](evidence/ui/embedded-local/gateway-adapter.patch), recorded without context lines. Apply it with `git apply --unidiff-zero` from the matching gateway checkout. The original Lens modules remain in the worktree during browser qualification. Before removing them, migrate the trace types imported by `components/networking.tsx`, the type assertions in `lib/http/api.test-d.ts`, and the old Lens test helpers. The page no longer mounts that copied workspace

## Pack and install

Set these paths to the Lens checkout and an isolated LiteLLM checkout containing the adapter diff. Stop only your own frontend before replacing its dependencies

```sh
lens_repo=/path/to/lens
gateway_repo=/path/to/litellm
qualification_dir=$(mktemp -d "${TMPDIR:-/tmp}/lens-embedded.XXXXXX")

cd "$lens_repo"
npm ci --ignore-scripts --no-audit --no-fund
npm pack --workspace src/ui/lib --pack-destination "$qualification_dir" --json > "$qualification_dir/package.json"
shasum -a 256 "$qualification_dir"/*.tgz

cd "$gateway_repo/ui/litellm-dashboard"
npm ci --ignore-scripts --no-audit --no-fund
npm install --no-save --package-lock=true --ignore-scripts --no-audit --no-fund "$qualification_dir"/*.tgz
git diff --exit-code -- package.json package-lock.json
```

Keep lockfile reading enabled. A scratch qualification confirmed that these install commands leave the existing lockfile byte-identical and preserve every installed dependency version already recorded in it. `--package-lock=false` instead re-resolved caret dependencies during the initial local attempt, so that attempt is excluded from final browser evidence. The development tarball stays outside the repository and is not a releasable dependency declaration

Record the package integrity from `package.json`, source revisions and command results beside the screenshots. The checked development tarball has SHA-256 `926f0a2eb0f2ea0ab3829d9a51e66c37804e669cd18e01016ee9ec72cd772b66`; a later source change must produce new artifact evidence

## Focused checks

```sh
cd "$lens_repo"
npm run typecheck:ui
npm exec --workspace @litellm/lens-ui -- vitest run src/lib/http/configure.test.ts src/lib/http/sessionRequests.test.ts

cd "$gateway_repo/ui/litellm-dashboard"
npm run typecheck
npx vitest run src/components/lens/EmbeddedLens.test.tsx 'src/app/(dashboard)/lens/page.test.tsx'
npx eslint src/components/lens/EmbeddedLens.tsx src/components/lens/EmbeddedLens.test.tsx 'src/app/(dashboard)/lens/page.tsx' 'src/app/(dashboard)/lens/page.test.tsx'
```

The shared HTTP tests exercise prefix changes after configuration, static/default prefix fallback, token refresh and logout. The adapter tests exercise session and read-only propagation, current custom headers and API origins, gateway error handling, and native spend-log lookup/drawer delegation. These tests complement live browser checks; they do not establish full UI or backend parity

Preserved outputs are [shared HTTP tests](evidence/ui/embedded-local/shared-http-tests.log), [shared typecheck](evidence/ui/embedded-local/shared-typecheck.log), [gateway tests](evidence/ui/embedded-local/gateway-tests.log), [gateway typecheck](evidence/ui/embedded-local/gateway-typecheck.log), and [gateway lint](evidence/ui/embedded-local/gateway-lint.log)

## Run the frontend

The local gateway must already serve authenticated APIs at `127.0.0.1:4000`. Its isolated config, database, Lens connection and test identities are prerequisites. For this qualification the other services use Rust Lens on `4318`, ClickHouse on `18124`, and gateway Postgres on `15432`; standalone Lens does not acquire a Postgres dependency

When starting the gateway from a reused Python environment, include both the chosen worktree root and its `litellm-proxy-extras` directory on `PYTHONPATH`. Verify that `litellm` and `litellm_proxy_extras` resolve from the intended worktree before boot; the first local attempt resolved migration extras from an older checkout. Do not infer that a successful Python import means all package roots match

```sh
cd "$gateway_repo/ui/litellm-dashboard"
LENS_DEV_PROXY_URL=http://127.0.0.1:4000 npm run dev -- --hostname 127.0.0.1 --port 3000
```

The development proxy distinguishes JSON API requests from page navigation at `/lens`. Login through `http://127.0.0.1:3000/ui/login`, then open `http://127.0.0.1:3000/ui/lens`. Preserve the separate standalone preview on `3100`

## Browser acceptance

Use the real gateway login and an isolated account. Do not inject a master token or shared backend credential into the browser to make a failed flow pass

1. Open `/ui/lens?demo=true&agent=support_agent&tab=findings&issue=support:failed-lookups`, inspect the finding, open its trace evidence, navigate back, and reload the deep link. Verify the selected agent, finding and tab survive and gateway navigation remains usable
2. Turn demo data off. Send a real provider request through the gateway, then ingest a trace whose legitimate owner and request ID match that request. Open its span, follow the spend-log link, and verify the native gateway drawer shows exactly that request. Return to the same trace with the drawer's back action
3. Verify missing or mismatched spend requests do not open an unrelated request. Check a viewer can read permitted data while mutation controls and server authorization remain restricted. Verify an unauthenticated session returns to login, token refresh takes effect, and logout clears access
4. Repeat the relevant navigation behind a non-root gateway prefix and inspect asset/link destinations. Compare traces, findings, investigations, datasets and settings at desktop and narrow widths in light and dark themes. Record exact URLs, actions and screenshots for each flow qualified

Real ingestion, a paid provider request, storage read-back and the correlated native drawer are separate evidence from synthetic demo navigation. An HTTP 200 for the Next page only establishes compilation and serving

## Recorded local results

The locked-dependency candidate passed normal administrator login, synthetic OTLP ingestion through the Rust service into ClickHouse, and trace inspection in the shared UI. A real provider completion through the local gateway returned `Lens local integration works`. The legitimately matching trace displayed its recorded cost and opened LiteLLM's native request-log drawer with the matching request ID, model, tokens and cost. The drawer's back action returned to that trace

Reloading the full trace URL reopened the selected trace and loaded its input/output. Clicking the account menu outside the inspector intentionally clears its URL state and starts the closing animation, matching the original gateway implementation. This is distinct from a reload losing its deep link

A second ingested trace with the same trace/span IDs but a different tenant did not receive the provider spend join. A normal viewer login displayed read-only Lens, with settings and dataset write actions unavailable. With the viewer's authenticated API key, dataset reads returned 200 and creation returned 403. Anonymous Lens and trace reads returned 401. Logout returned the browser to the gateway login page

The screenshots show [the native log drawer](evidence/ui/embedded-local/native-log-drawer.png), [the reloaded trace](evidence/ui/embedded-local/trace-deep-link.png), and [the viewer dataset screen](evidence/ui/embedded-local/viewer-datasets.png). These checks establish the exercised local paths only. The full visual, role, prefix, expiry, browser-navigation and feature matrix remains open. The gateway API still owns investigations and other unported features; no standalone Rust readiness is implied

## Existing Playwright harness

LiteLLM's browser package is `tests/e2e/ui`, with pinned `@playwright/test` in its own lockfile. It accepts `E2E_UI_BASE_URL` and `E2E_UI_ARTIFACT_DIR`; existing relevant specs include `tests/login/login.spec.ts`, `tests/auth/unauthenticatedRedirect.spec.ts`, `tests/auth/logout.spec.ts` and `tests/logs/logs.spec.ts`. No Lens-specific Playwright spec existed at the audited main revision

Its default `globalSetup.ts` changes UI settings, seeds role users and writes authentication storage state. `run_e2e.sh` starts its own seeded Postgres, mock provider and proxy. Run it only against the disposable stack it owns, with distinct ports when another stack is running. It is not a read-only smoke command for the current developer gateway

The script also runs `npm install` in the dashboard, which can remove the temporary, unsaved Lens package. The following focused harness command becomes applicable after the immutable Lens dependency is declared, or after a local harness install step is explicitly adapted to reinstall the checked tarball. Use the manual browser sequence above for the current temporary package candidate

```sh
cd "$gateway_repo/tests/e2e/ui"
npm ci --ignore-scripts --no-audit --no-fund
PROXY_PORT=4100 POSTGRES_PORT=15532 MOCK_LLM_PORT=8190 MOCK_PRESIDIO_PORT=8191 \
  ./run_e2e.sh tests/login/login.spec.ts tests/auth/unauthenticatedRedirect.spec.ts tests/auth/logout.spec.ts tests/logs/logs.spec.ts
```

The existing harness's mock provider qualifies dashboard wiring only. Lens browser scenarios still need dedicated specs for deep links, permission boundaries and the correlated trace-to-native-log flow, with real provider evidence retained separately
