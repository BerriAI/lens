# Deploy Lens on Render

The root [Render Blueprint](../render.yaml) deploys the Lens UI, API, and shared GitHub App service together, with ClickHouse on Render's private network. Both services use the `1c-2g` compute plan in Oregon. ClickHouse keeps database and Keeper data on a 10 GB disk at `/var/lib/clickhouse`; it has no public endpoint

Create a Blueprint in the [Render dashboard](https://dashboard.render.com/) from this repository. Enter the GitHub App slug, client ID, client secret, and full PEM private key when prompted. The Blueprint generates the Lens admin token and database password and derives `LENS_PUBLIC_URL` from Render's assigned HTTPS URL. Do not set `LENS_GITHUB_SERVICE_URL` on this service, because it operates the App itself

Wait for ClickHouse to start, then check that the Lens service passes `/health/ready`. If Lens's first deploy starts before ClickHouse is ready, redeploy Lens after the database is running. In the Lens service's Environment settings, save `LENS_ADMIN_TOKEN` in your password manager and use it to sign in. Preserve this token across redeploys and alongside backups because Lens uses it to encrypt saved GitHub connection credentials

## Finish the GitHub App connection

Open the GitHub App settings and use the Lens service's public origin for its user authorization callback URL and setup URL:

```text
https://<your-service>.onrender.com/lens/github/callback
https://<your-service>.onrender.com/lens/github/setup
```

Enable **Redirect on update** and leave **Request user authorization (OAuth) during installation** and webhooks disabled. See [GitHub App settings and permissions](github-app.md) for the required repository permissions

Open Lens, send your agent's first trace, and choose **Connect GitHub** for that agent. Install or authorize the App, select the repository, and return to Lens. Choose **Set up PR evals**, follow the [eval workflow guide](github-evals.md), and run a baseline before opening a PR. A successful deployment alone does not run evaluations; the repository's workflow runs them and Lens publishes the results

Other self-hosted Lens instances can connect through this service by setting `LENS_GITHUB_SERVICE_URL` to its public origin and `LENS_PUBLIC_URL` to their own Lens address. GitHub App secrets remain on this shared service

## Operations

Validate configuration changes with `render blueprints validate render.yaml`. The Blueprint waits for passing repository checks before automatically deploying new commits. Keep one Lens instance until the [shared-service rate limit and authorization retention requirements](github-app.md#operate-the-official-lens-app-service) are handled for your deployment

Tracing and GitHub connections do not require a model provider. Configure [analysis models](analysis.md) for investigations and an [evaluation provider](signals.md) for signals. Before relying on investigation calculations, qualify the [calculation sandbox](sandbox.md) on the deployed Render host; a passing `/health/ready` check does not verify its Landlock and seccomp support

Use ClickHouse-native backups and test restores; Render disk snapshots are not a database backup strategy. Database redeploys interrupt access while the disk moves to the replacement instance. Increase compute or disk capacity as trace volume grows

If you add a custom domain, update `LENS_PUBLIC_URL` in the Blueprint and change both GitHub App return URLs to that origin. Existing self-hosted clients also need the new service origin. Never put App credentials or admin tokens in this file or source control

Render documents the [Blueprint fields](https://render.com/docs/blueprint-spec), [private services](https://render.com/docs/private-services), and [ClickHouse deployment and backups](https://render.com/docs/deploy-clickhouse)
