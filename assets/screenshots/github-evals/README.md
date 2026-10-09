# GitHub App connection browser evidence

Captured from the Lens UI on October 9, 2026 at a 1440 × 1000 desktop viewport and a 390 × 844 mobile viewport

`after-github-app-unconfigured.png` shows the real local Lens backend with no GitHub App credentials configured. The frontend connected directly to that backend, without the response fixture, and correctly offered the administrator setup guide and configuration retry

The current App flow appears in `after-github-repositories-fixture.png`, `after-github-connected-fixture.png` and `after-github-connected-mobile-fixture.png`. These capture the real rendered Lens UI with simulated responses for Lens’s GitHub connection API through a temporary loopback proxy. All other requests go to an actual local Lens server and an isolated ClickHouse database. No production configuration or code was added for the proxy

The repository picker selects a synthetic repository, then Connect repository submits its selected ID and removes the authorization parameter from the return URL. The connected screenshots show the resulting UI state. These screenshots do not establish real GitHub consent, token exchange, installation access or connection persistence in Lens

`after-github-eval-setup-fixture.png` and `after-github-app-workflow-fixture.png` show the subsequent eval setup using that simulated connection. The repository is read-only, and the generated workflow publishes through the App with a contents-read-only GitHub token. The Lens URL is an example. No provider was called, no GitHub workflow ran, and no PR comment was posted

The agent, trace, dataset and eval named `github-flow-qa` are synthetic QA fixtures created through Lens’s public APIs. Frontend integration tests separately cover authorization requests, callback routing, repository selection, connection refresh, lost access, disconnect, expired authorization, failed requests and subsequent eval setup

`before-agents.png`, `before-agents-demo.png` and `before-evals.png` show the original product. The earlier `after-agents.png`, `after-connect.png`, `after-workflow.png`, `after-workflow-credentials.png`, `after-verify-waiting.png` and `after-mobile.png` are historical captures of the workflow-only implementation, before the GitHub App connection was added. They are retained for comparison and do not represent the final primary connection flow
