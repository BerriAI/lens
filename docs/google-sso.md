# Google Workspace sign-in

The Rust backend supports Google OpenID Connect authorization code sign-in. Create a Google OAuth web application and register exactly `https://<lens-host>/auth/google/callback` as its authorized redirect URI. Configure the OAuth consent screen for your organization

Set `LENS_PUBLIC_URL` to the externally visible Lens origin, and set these values in the backend environment (the deployment's `deploy/lens/.env` is passed to the container)

```dotenv
LENS_PUBLIC_URL=https://lens.example.com
LENS_GOOGLE_CLIENT_ID=your-web-client-id.apps.googleusercontent.com
LENS_GOOGLE_CLIENT_SECRET=your-client-secret
LENS_GOOGLE_ALLOWED_DOMAINS=example.com,example.org
```

Keep the client secret in your deployment's secret store. Do not expose it through frontend configuration. All three Google values must be configured together. Empty values disable Google sign-in only when all three are empty. Invalid or partial configuration prevents startup. The domain list accepts exact DNS names, is case insensitive, and does not accept wildcards. Both Google's signed `hd` claim and the verified email's domain must match. Gmail consumer accounts are not accepted

`LENS_PUBLIC_URL` must be an HTTPS origin without credentials, path prefixes, query, or fragment. HTTP is supported only for `localhost`, `127.0.0.1`, or `[::1]` development. Configure the reverse proxy to forward `/auth/*` to the Rust backend and preserve response cookies. The configured public origin, rather than forwarded request headers, determines the callback URL

`GET /auth/google/config` returns only `{"enabled":true}` or `{"enabled":false}`. Navigate the browser to `GET /auth/google/start?return_to=<encoded-local-path>`. Encode the entire destination, including its query and fragment, as the `return_to` parameter. The callback redirects to the original safe local destination. Failure adds `sso_error=failed` before its fragment. Invalid destinations or callbacks without a valid browser-bound state fall back to `/`. Authentication endpoints cannot be used as return destinations

Google sign-in uses PKCE S256, a nonce, and an HttpOnly SameSite=Lax browser cookie containing AES-256-GCM authenticated encrypted login state. HTTPS uses a Secure `__Host-` cookie to prevent sibling subdomains from planting the browser binding. The random OAuth state must match the encrypted cookie. State expires after ten minutes measured by the process's monotonic clock. Starting a login reserves no server-side slot, so abandoned starts cannot exhaust pending login capacity. Destinations whose encrypted cookie would exceed the cookie size budget fall back to `/`

Google's single-use authorization code exchange and PKCE prevent authorization-code reuse. After token signature, claims, nonce, and domain validation, an atomic replay cache admits at most one session creation per login state, including concurrent callbacks. Failed callbacks clear the browser cookie but do not reserve replay-cache entries. The replay cache retains successful redemptions until state expiry, is bounded to 1,024 entries per process, and fails closed when full without evicting live entries. Up to eight exchanges can run concurrently. The encryption key and replay cache are process-local: restarts invalidate in-flight logins, and multi-replica deployments must route start and callback to the same replica. Completed sessions use the existing shared ClickHouse session store and last eight hours. They survive restarts and support existing sign-out and origin checks. Google access and refresh tokens are not retained

Google ID tokens must have a valid RS256 signature from Google's HTTPS JWKS endpoint, an accepted Google issuer, this application's audience and authorized party, valid timestamps, and the original nonce. Provider HTTP redirects are refused. Token and key responses are size bounded and requests time out after fifteen seconds. Keys are fetched for each completed code exchange so rotation does not require a restart; Google connectivity is required during login

Google users receive the existing `proxy_admin_viewer` role with the stable user ID `google:<Google subject>` and read access to all deployment traces, so teammates can open shared trace links. Configure the domain allowlist only for organizations trusted to read this deployment's data. This does not link Google email addresses to existing gateway users or grant administrator write permissions. Existing setup-token sessions retain administrator access. Changing the domain allowlist affects new sign-ins; existing sessions expire or must be revoked separately

The sign-in page displays **Sign in with SSO** when Google is configured and preserves the original path, query, and fragment. Setup-token sign-in and demo access remain available. Failed sign-in shows a generic error and allows retrying
