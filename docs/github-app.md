# Connect GitHub to an agent

Choose **Connect GitHub** after adding your agent or from its row in **Agents**. Lens opens the configured GitHub App’s installation page, where GitHub lets you choose an account and repository access. Lens then verifies your authorization and returns you to a repository picker. Choose **Connect repository** to save the connection to that agent

If a repository is missing, choose **Install GitHub App** or **Manage App access** to update the installation. An organization owner may need to approve an installation request before the repository becomes available

Returning to **Connect GitHub** shows the saved repository. **Manage repository access** opens GitHub’s installation flow. **Disconnect** removes the link from this agent in Lens; uninstall the App in GitHub if you also want to revoke its installation

After connecting, choose **Set up PR evals** to configure the agent’s eval workflow. Repository authorization and running your agent are separate steps: Lens still needs an eval task that runs the checked-out agent and returns its trace. See the [PR eval setup guide](github-evals.md)

## Configure the GitHub App once

Register a GitHub App for your Lens deployment. The deployment administrator configures it once; people connecting agents authorize and select repositories in GitHub

For a deployment at `https://lens.example.com`, set the homepage URL to `https://lens.example.com/ui/`, the user authorization callback URL to `https://lens.example.com/lens/github/callback`, and the setup URL to `https://lens.example.com/lens/github/setup`. Enable **Redirect on update**. Leave **Request user authorization (OAuth) during installation** disabled: Lens starts OAuth after verifying the installation return

The App needs repository metadata read access, contents read access, Actions read access, pull requests write access, and checks write access to discover repositories, verify workflow runs, and publish eval reports. The workflow executes in GitHub Actions, so this flow does not require pull request webhooks or a Lens webhook endpoint

Set these values only in the Lens server’s environment or secret manager:

| Variable | Value |
| --- | --- |
| `LENS_PUBLIC_URL` | Public Lens origin, such as `https://lens.example.com` |
| `LENS_GITHUB_APP_SLUG` | App slug from its `github.com/apps/…` URL |
| `LENS_GITHUB_CLIENT_ID` | GitHub App client ID |
| `LENS_GITHUB_CLIENT_SECRET` | GitHub App client secret |
| `LENS_GITHUB_PRIVATE_KEY` | Complete PEM private key generated for the App |

The Compose deployment loads these from its existing private `deploy/lens/.env` file. For Kubernetes, put the variables in a Secret and reference it with the chart’s `extraEnvFrom` setting. Restart Lens after configuring the App. Missing configuration appears as **GitHub App setup required** in the connection dialog

Keep the client secret and private key out of browser configuration, repository files, screenshots, and PR descriptions. Lens exchanges the OAuth code on the server and discards the user access token after discovering accessible repositories. The durable connection stores repository and installation identifiers, not the user token

Authorization expires after ten minutes, is bound to the initiating Lens identity and browser, and can be used only once. If it expires or is cancelled, choose **Try GitHub again**. Lens verifies repository access with GitHub before saving it and checks the installation when showing a saved connection

GitHub documents the [user authorization flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app) and the distinction between [setup and callback URLs](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/about-the-setup-url)
