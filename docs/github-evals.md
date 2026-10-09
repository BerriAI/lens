# Connect an agent to GitHub

After Lens receives your agent’s first trace, choose **Connect GitHub**. Authorize the GitHub App, choose a repository, and save the connection. Then choose **Set up PR evals**. You can also open the connection from the agent’s row in **Agents**, its workspace header, or an eval with no runs. The [GitHub App guide](github-app.md) covers authorization and deployment configuration

The connected GitHub repository is preselected. Choose an eval for that agent and enter a Lens HTTPS address reachable by GitHub-hosted runners. Create a dataset and eval first if the agent has none. The optional runtime fields accept an existing async Python task and the command needed to install or start your agent

The setup gives you a workflow, `[tool.lens]` configuration, and an eval adapter. Merge these into the agent repository without replacing existing configuration. If you did not provide a task function, implement the adapter before running it. The adapter accepts a `lens.Case`, starts the agent, and returns a `lens.Run` containing its trace reference. The [SDK guide](../src/sdk/README.md#complete-http-agent-example) shows the HTTP integration and required trace attributes

Run the agent built from the checked-out commit. Its traces must identify the selected agent, set `deployment.environment` to `lens-eval`, and use its actual build SHA as `agent.version`. Add its dependencies, startup steps, and model or service credentials to the workflow as needed

Add `LENS_API_KEY` as a GitHub Actions repository secret and `LENS_BASE_URL` as a repository variable. The key needs dataset and eval access; a tracing-only key is insufficient. Map each additional agent secret to an environment variable on the Lens Action and any agent startup step. Setup links directly to the repository’s secret and variable settings. Lens never asks for a GitHub token in the browser. The generated Action uses `report-via-app: true` to publish through the connected GitHub App

The private preview Action must be accessible to the consuming repository through the organization’s GitHub Actions policy. The generated workflow explicitly builds the SDK from the Action’s pinned source checkout, so it does not depend on an unpublished wheel release. GitHub-hosted runners provide the required Rust installer

Commit the workflow and adapter to `main` to establish a baseline, or select **Run workflow** on `main` in Actions. Opening, reopening, or updating a same-repository pull request targeting `main` then runs the selected eval. Fork PRs are skipped because the eval needs credentials. A missing baseline is reported, and absolute gate conditions still apply

Return to **Verify PR eval** and choose **Check for PR eval**. Lens matches the agent, eval, and repository in the run’s GitHub Actions URL. It shows running, failed, or completed results and links to the run, workflow, and PR. A completed eval is not confirmation that GitHub published its comment: check the workflow’s **Publish report** step and the PR itself

The report includes case pass counts, regressions, cost, and links to Lens. The connected GitHub App posts a report for each eval run; retrying publication updates that run’s report. A failed gate fails the GitHub job and check. Existing workflows without `report-via-app` retain the GitHub Actions report publisher

GitHub App authorization saves the agent’s repository connection. The separate eval setup configures GitHub Actions through reviewed repository files; connecting a repository alone does not execute the agent
