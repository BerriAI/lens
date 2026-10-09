# Connect GitHub to an agent

Lens uses one official GitHub App. Customers install it on their repositories, and that App posts eval results on pull requests. The App credentials stay on the hosted Lens GitHub service, including when the customer runs Lens on their own infrastructure

Choose **Connect GitHub** after Lens receives your agent’s first trace, from its row in **Agents**, or from its workspace header. Install the Lens App in your GitHub account or organization, choose repository access, and authorize the connection. Confirm the repository and destination Lens instance before returning to Lens

Returning to **Connect GitHub** shows the saved repository. **Manage repository access** opens the installation flow. **Disconnect** revokes the connection from this Lens instance; uninstall the App in GitHub if you also want to revoke its repository installation

After connecting, choose **Set up PR evals** to configure the agent’s eval workflow. Lens needs an eval task that runs the checked-out agent and returns its trace. See the [PR eval setup guide](github-evals.md). After an eval completes, the Lens App posts the result on its PR with links to the run

## Self-hosted Lens

Point `LENS_GITHUB_SERVICE_URL` at the official Lens GitHub service and set `LENS_PUBLIC_URL` to the address people use to reach your Lens instance. The service address will be published when the official App is deployed. Customers do not register an App or configure GitHub client secrets or private keys

The browser returns to your configured Lens address after authorization. Lens redeems a one-time code on the server and saves a credential restricted to that repository connection. It cannot use that credential to make arbitrary GitHub API calls or obtain the App’s private key or installation token

The saved credential is encrypted using a key derived from `LENS_ADMIN_TOKEN`. Preserve this deployment secret with your database backups and across restarts. Changing it makes previous credentials unreadable; reconnecting alone cannot revoke that previous access. Restore the original secret or revoke the GitHub App installation to retire it

Replacing or disconnecting a connection queues old credentials for revocation. Lens retains them encrypted and retries if the service is unavailable. **Check connection again** retries cleanup; the connection shows that access needs attention until cleanup succeeds

Reports are read from completed runs in your Lens instance, then the service verifies the GitHub workflow, PR and commit before publishing

The eval runner must be able to reach your Lens instance. For a private deployment, use a runner with access to its network. This integration does not need inbound GitHub webhooks; the hosted service never fetches customer Lens URLs

## Operate the official Lens App service

This setup is for the operator of the shared service and happens once. Register the official Lens GitHub App under the Lens organization, make it installable by other accounts, and deploy the service at an owned HTTPS address

For a service at `https://github-service.example.com`, set the user authorization callback URL to `https://github-service.example.com/lens/github/callback` and the setup URL to `https://github-service.example.com/lens/github/setup`. The App homepage can link to the Lens repository. Enable **Redirect on update**. Leave **Request user authorization (OAuth) during installation** disabled because the service starts OAuth after verifying the installation return

The App needs repository metadata read access, contents read access, Actions read access, pull requests write access, and checks write access. These allow it to discover repositories, verify workflow runs, and publish eval comments and checks. Disable webhooks for this flow because GitHub Actions executes the eval

Set these values only on the hosted service:

| Variable | Value |
| --- | --- |
| `LENS_PUBLIC_URL` | The service’s public HTTPS origin |
| `LENS_GITHUB_SERVICE_ENABLED` | `true` to enable connections from self-hosted instances |
| `LENS_GITHUB_APP_SLUG` | Official App slug from its `github.com/apps/…` URL |
| `LENS_GITHUB_CLIENT_ID` | GitHub App client ID |
| `LENS_GITHUB_CLIENT_SECRET` | GitHub App client secret |
| `LENS_GITHUB_PRIVATE_KEY` | Complete PEM private key generated for the App |

Use the existing Lens deployment, authentication and ClickHouse settings alongside these values. The Compose deployment loads environment values from its private `deploy/lens/.env` file. For Kubernetes, use a Secret with the chart’s `extraEnvFrom` setting. Restart the service after changing its environment

Before exposing the service publicly, configure rate limits at the ingress and retention for expired authorization records. The server admits at most 30 new handoffs per minute per process; this does not replace limits shared across replicas. Monitor authorization storage growth because expiration rejects old handoffs but does not delete their stored records

Keep the client secret and private key out of browser configuration, customer deployments, repository files, screenshots and PR descriptions. The service exchanges OAuth codes on the server and discards user access tokens after discovering accessible repositories

Authorization is bound to the initiating browser and Lens connection, expires, and can be used only once. The service verifies repository access rather than trusting the installation ID in a redirect. Organization policies may require an owner to approve installation before a repository becomes available

Direct App configuration without `LENS_GITHUB_SERVICE_ENABLED` remains available for private deployments that operate their own App, but it is not required for customers using the official service

GitHub documents [registering an App](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/registering-a-github-app), the [user authorization flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app), and [setup URLs](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url)
