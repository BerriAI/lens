import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import {
  chooseSelectOption,
  testQueryClient,
} from "../../../../tests/test-utils";
import { AgentGitHubDialog } from "./AgentGitHubDialog";
import type {
  GitHubAuthorization,
  GitHubConnectionStatus,
  GitHubRepositoryOption,
} from "./githubConnection";

const agent = "qa-agent";
const authorizationId = "qa-authorization";
const authorizationPath = `/lens/github/authorizations/${authorizationId}`;
const connectionPath = `/lens/github/connections/${agent}`;
const authorizationUrl =
  "https://github.com/login/oauth/authorize?client_id=qa-client&state=qa-state";
const installationUrl =
  "https://github.com/apps/lens-qa/installations/new?state=qa-state";
const authorizationStart = {
  authorization_url: authorizationUrl,
  authorization_id: authorizationId,
  expires_at: "2026-10-09T20:00:00Z",
};
const repository: GitHubRepositoryOption = {
  id: 101,
  full_name: "lens-test/agent",
  installation_id: 17,
  default_branch: "main",
};
const anotherRepository: GitHubRepositoryOption = {
  ...repository,
  id: 102,
  full_name: "lens-test/another-agent",
};
const connection: GitHubConnectionStatus = {
  agent,
  repository_id: repository.id,
  repository: repository.full_name,
  installation_id: repository.installation_id,
  default_branch: repository.default_branch,
  connected_at: "2026-10-09T19:00:00Z",
  available: true,
};
const disconnected = {
  configured: true,
  app_slug: "lens-qa",
  connection: null,
};
const ready: GitHubAuthorization = {
  status: "ready",
  repositories: [repository, anotherRepository],
};

let gateway = stubGateway();

beforeEach(() => {
  testQueryClient.clear();
  gateway = stubGateway();
  gateway.get.mockImplementation((path) => {
    if (path === "/lens/github/status") return disconnected;
    if (path === authorizationPath) return ready;
    throw new Error(`Unexpected GET ${path}`);
  });
  gateway.post.mockImplementation((_path, options) => ({
    ...authorizationStart,
    authorization_url:
      typeof options.body === "object" &&
      options.body !== null &&
      "install" in options.body &&
      options.body.install === true
        ? installationUrl
        : authorizationUrl,
  }));
});

describe("GitHub App connection", () => {
  it("should wait for an explicit choice after configuration becomes available", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.get.mockReturnValue({
      ...disconnected,
      configured: false,
      app_slug: null,
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );

    expect(
      await screen.findByRole("heading", {
        name: "GitHub connection unavailable",
      }),
    ).toBeVisible();
    expect(
      screen.getByRole("link", { name: "GitHub connection guide" }),
    ).toHaveAttribute("href", expect.stringContaining("/docs/github-app.md"));
    expect(
      screen.queryByRole("textbox", { name: "GitHub repository" }),
    ).not.toBeInTheDocument();
    expect(gateway.post).not.toHaveBeenCalled();
    expect(onAuthorize).not.toHaveBeenCalled();

    gateway.get.mockReturnValue(disconnected);
    await user.click(
      screen.getByRole("button", { name: "Check configuration" }),
    );
    await screen.findByRole("button", { name: "Install GitHub App" });
    expect(onAuthorize).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Install GitHub App" }));
    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledExactlyOnceWith(installationUrl),
    );
    expect(gateway.post).toHaveBeenCalledExactlyOnceWith(
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install: true } }),
    );
  });

  it.each([
    ["Install GitHub App", true, installationUrl],
    ["I already installed it", false, authorizationUrl],
  ] as const)("should wait for the user to choose %s before opening GitHub", async (label, install, destination) => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    renderWithLens(
      <AgentGitHubDialog agent={agent} onOpenChange={vi.fn()} onAuthorize={onAuthorize} />,
    );
    const choice = await screen.findByRole("button", { name: label });
    expect(choice).toBeEnabled();
    expect(gateway.post).not.toHaveBeenCalled();
    expect(onAuthorize).not.toHaveBeenCalled();
    await user.click(choice);
    await waitFor(() => expect(onAuthorize).toHaveBeenCalledExactlyOnceWith(destination));
    expect(gateway.post).toHaveBeenCalledExactlyOnceWith(
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install } }),
    );
    expect(screen.queryByRole("textbox", { name: "Lens URL" })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Set up PR evals" })).not.toBeInTheDocument();
    await testQueryClient.invalidateQueries();
    expect(gateway.post).toHaveBeenCalledOnce();
    expect(onAuthorize).toHaveBeenCalledOnce();
  });

  it("retries the installation picker when the initial connection request fails", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.post.mockImplementationOnce(() => {
      throw new Error("Could not start GitHub installation");
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "Install GitHub App" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not start GitHub installation",
    );
    expect(onAuthorize).not.toHaveBeenCalled();
    await user.click(
      screen.getByRole("button", { name: "Install GitHub App" }),
    );
    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledExactlyOnceWith(installationUrl),
    );
    expect(gateway.post).toHaveBeenNthCalledWith(
      2,
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install: true } }),
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    await testQueryClient.invalidateQueries();
    expect(gateway.post).toHaveBeenCalledTimes(2);
  });

  it("opens the official connection service returned by the agent status", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    const serviceOrigin = "https://connections.example.test";
    const url = `${serviceOrigin}/authorize?state=qa-broker-state`;
    gateway.get.mockReturnValue({
      ...disconnected,
      service_origin: serviceOrigin,
    });
    gateway.post.mockReturnValue({
      ...authorizationStart,
      authorization_url: url,
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "Install GitHub App" }));

    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledExactlyOnceWith(url),
    );
    expect(gateway.post).toHaveBeenCalledExactlyOnceWith(
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install: true } }),
    );
    await testQueryClient.invalidateQueries();
    expect(onAuthorize).toHaveBeenCalledOnce();
  });

  it("blocks an untrusted connection redirect and retries without navigating away", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    const serviceOrigin = "https://connections.example.test";
    const trustedUrl = `${serviceOrigin}/authorize?state=qa-broker-state`;
    gateway.get.mockReturnValue({
      ...disconnected,
      service_origin: serviceOrigin,
    });
    gateway.post
      .mockReturnValueOnce({
        ...authorizationStart,
        authorization_url:
          "https://connections.example.test.untrusted.example.test/authorize",
      })
      .mockReturnValue({
        ...authorizationStart,
        authorization_url: trustedUrl,
      });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "Install GitHub App" }));

    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not open GitHub. Try again",
    );
    expect(onAuthorize).not.toHaveBeenCalled();
    await testQueryClient.invalidateQueries();
    expect(gateway.post).toHaveBeenCalledOnce();
    await user.click(
      screen.getByRole("button", { name: "Install GitHub App" }),
    );
    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledExactlyOnceWith(trustedUrl),
    );
    expect(gateway.post).toHaveBeenNthCalledWith(
      2,
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install: true } }),
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("links only the selected authorized repository and remembers it when reopened", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    const onAuthorizationComplete = vi.fn();
    const chosen: GitHubConnectionStatus = {
      ...connection,
      repository_id: anotherRepository.id,
      repository: anotherRepository.full_name,
    };
    gateway.put.mockImplementation(() => {
      gateway.get.mockImplementation((path) =>
        path === "/lens/github/status"
          ? { ...disconnected, connection: chosen }
          : ready,
      );
      return chosen;
    });
    const view = renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        authorizationId={authorizationId}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
        onAuthorizationComplete={onAuthorizationComplete}
      />,
    );
    const picker = await screen.findByRole("combobox", {
      name: "GitHub repository",
    });
    await chooseSelectOption(user, picker, anotherRepository.full_name);
    await user.click(
      screen.getByRole("button", { name: "Connect repository" }),
    );

    expect(
      await screen.findByRole("heading", { name: "GitHub connected" }),
    ).toBeVisible();
    expect(gateway.put).toHaveBeenCalledExactlyOnceWith(
      connectionPath,
      expect.objectContaining({
        body: {
          authorization_id: authorizationId,
          repository_id: anotherRepository.id,
        },
      }),
    );
    expect(
      screen.getByRole("link", { name: anotherRepository.full_name }),
    ).toHaveAttribute(
      "href",
      `https://github.com/${anotherRepository.full_name}`,
    );
    expect(
      screen.getByText(/Connecting GitHub does not run an eval by itself/),
    ).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Set up PR evals" }),
    ).toBeEnabled();
    expect(onAuthorize).not.toHaveBeenCalled();
    expect(onAuthorizationComplete).toHaveBeenCalledOnce();

    view.unmount();
    testQueryClient.clear();
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    expect(
      await screen.findByRole("heading", { name: "GitHub connected" }),
    ).toBeVisible();
    expect(
      screen.getByRole("link", { name: anotherRepository.full_name }),
    ).toBeVisible();
    expect(onAuthorize).not.toHaveBeenCalled();
  });

  it("waits for the callback before offering repository selection", async () => {
    const onAuthorize = vi.fn();
    gateway.get.mockImplementation((path) =>
      path === authorizationPath
        ? { status: "pending", repositories: [] }
        : disconnected,
    );
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        authorizationId={authorizationId}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    expect(
      await screen.findByText("Completing GitHub authorization"),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Connect repository" }),
    ).not.toBeInTheDocument();
    expect(onAuthorize).not.toHaveBeenCalled();
    gateway.get.mockImplementation((path) =>
      path === authorizationPath ? ready : disconnected,
    );
    await testQueryClient.invalidateQueries();
    expect(
      await screen.findByRole("combobox", { name: "GitHub repository" }),
    ).toBeVisible();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("restarts cancelled or expired authorization without claiming the agent is connected", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.get.mockImplementation((path) =>
      path === authorizationPath
        ? { status: "failed", repositories: [], error: "Authorization expired" }
        : disconnected,
    );
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        authorizationId={authorizationId}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Your agent is not connected",
    );
    expect(
      screen.queryByRole("heading", { name: "GitHub connected" }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Try GitHub again" }));
    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledWith(authorizationUrl),
    );
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install: false } }),
    );
  });

  it("requests App installation when authorization has no accessible repositories", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.get.mockImplementation((path) =>
      path === authorizationPath
        ? { status: "ready", repositories: [] }
        : disconnected,
    );
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        authorizationId={authorizationId}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Install GitHub App" }),
    );
    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledWith(installationUrl),
    );
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install: true } }),
    );
    expect(gateway.put).not.toHaveBeenCalled();
  });

  it("offers access recovery and blocks eval setup when the App loses repository access", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.get.mockReturnValue({
      ...disconnected,
      connection: {
        ...connection,
        available: false,
        availability_error: "Installation unavailable",
      },
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "GitHub access needs attention",
    );
    expect(
      screen.queryByRole("button", { name: "Set up PR evals" }),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("button", { name: "Restore GitHub access" }),
    );
    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledWith(installationUrl),
    );
    expect(gateway.post).toHaveBeenCalledWith(
      "/lens/github/authorize",
      expect.objectContaining({ body: { agent, install: true } }),
    );
  });

  it("rechecks pending access cleanup without starting another GitHub authorization", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.get.mockReturnValue({
      ...disconnected,
      connection: {
        ...connection,
        available: false,
        availability_error: "Previous access could not be revoked. Try again",
      },
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Previous access could not be revoked",
    );
    gateway.get.mockReturnValue({ ...disconnected, connection });
    await user.click(
      screen.getByRole("button", { name: "Check connection again" }),
    );
    expect(
      await screen.findByRole("button", { name: "Set up PR evals" }),
    ).toBeEnabled();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(onAuthorize).not.toHaveBeenCalled();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("disconnects the persisted repository without automatically starting a new authorization", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.get.mockReturnValue({ ...disconnected, connection });
    gateway.delete.mockImplementation(() => {
      gateway.get.mockReturnValue(disconnected);
      return null;
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(
      await screen.findByRole("button", { name: "Install GitHub App" }),
    ).toBeVisible();
    expect(gateway.delete).toHaveBeenCalledWith(
      connectionPath,
      expect.anything(),
    );
    expect(
      screen.queryByRole("heading", { name: "GitHub connected" }),
    ).not.toBeInTheDocument();
    expect(onAuthorize).not.toHaveBeenCalled();
  });

  it("retries a failed status request before opening GitHub", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    gateway.get.mockImplementation(() => {
      throw new Error("Network unavailable");
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not check the GitHub connection",
    );
    expect(gateway.post).not.toHaveBeenCalled();
    gateway.get.mockReturnValue(disconnected);
    await user.click(screen.getByRole("button", { name: "Try again" }));
    await user.click(await screen.findByRole("button", { name: "Install GitHub App" }));
    await waitFor(() =>
      expect(onAuthorize).toHaveBeenCalledWith(installationUrl),
    );
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("keeps repository selection available when saving the connection is rejected", async () => {
    const user = userEvent.setup();
    gateway.put.mockImplementation(() => {
      throw new Error("Repository access changed. Authorize again");
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        authorizationId={authorizationId}
        onOpenChange={vi.fn()}
        onAuthorize={vi.fn()}
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Connect repository" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Repository access changed",
    );
    expect(
      screen.getByRole("combobox", { name: "GitHub repository" }),
    ).toBeEnabled();
    expect(
      screen.queryByRole("heading", { name: "GitHub connected" }),
    ).not.toBeInTheDocument();
  });

  it("does not start GitHub authorization when the Lens session is unauthorized", async () => {
    const onAuthorize = vi.fn();
    const fetch = vi
      .fn<typeof globalThis.fetch>()
      .mockResolvedValue(
        Response.json({ detail: "Not authenticated" }, { status: 401 }),
      );
    vi.stubGlobal("fetch", fetch);
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
      { accessToken: "" },
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not check the GitHub connection",
    );
    expect(
      screen.queryByRole("combobox", { name: "GitHub repository" }),
    ).not.toBeInTheDocument();
    expect(onAuthorize).not.toHaveBeenCalled();
    expect(fetch).toHaveBeenCalledOnce();
  });

  it("shows the callback repository picker when updating an existing connection", async () => {
    gateway.get.mockImplementation((path) =>
      path === authorizationPath ? ready : { ...disconnected, connection },
    );
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        authorizationId={authorizationId}
        onOpenChange={vi.fn()}
        onAuthorize={vi.fn()}
      />,
    );
    expect(
      await screen.findByRole("combobox", { name: "GitHub repository" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("heading", { name: "GitHub connected" }),
    ).not.toBeInTheDocument();
    expect(gateway.post).not.toHaveBeenCalled();
  });

  it("rechecks a cached disconnected agent before reopening authorization", async () => {
    const user = userEvent.setup();
    const onAuthorize = vi.fn();
    const view = renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    await user.click(await screen.findByRole("button", { name: "Install GitHub App" }));
    await waitFor(() => expect(onAuthorize).toHaveBeenCalledOnce());
    view.unmount();
    gateway.get.mockReturnValue({ ...disconnected, connection });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={onAuthorize}
      />,
    );
    expect(
      await screen.findByRole("heading", { name: "GitHub connected" }),
    ).toBeVisible();
    expect(onAuthorize).toHaveBeenCalledOnce();
  });

  it("carries the connected repository into eval setup and publishes through the App", async () => {
    const user = userEvent.setup();
    gateway.get.mockImplementation((path) => {
      if (path === "/lens/github/status")
        return { ...disconnected, connection };
      if (path === "/lens/datasets")
        return [
          {
            id: "qa-cases",
            name: "QA cases",
            agent_name: agent,
            revision: 1,
            case_count: 1,
            updated_at: "2026-10-09T19:00:00Z",
          },
        ];
      if (path === "/lens/evals")
        return [
          {
            name: "qa-eval",
            spec: {
              agent,
              dataset_id: "qa-cases",
              revision: 1,
              scorers: [{ kind: "task_completed" }],
              trials: 1,
              baseline: "main",
              gate: {
                regressions: 0,
                critical: 0,
                pass_rate: null,
                cost_per_case: null,
                min: {},
              },
              timeout_per_trial_ms: 1200000,
            },
            updated_at: "2026-10-09T19:00:00Z",
          },
        ];
      if (path === "/lens/evals/runs") return [];
      throw new Error(`Unexpected GET ${path}`);
    });
    renderWithLens(
      <AgentGitHubDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onAuthorize={vi.fn()}
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Set up PR evals" }),
    );
    expect(
      screen.getByRole("textbox", { name: "GitHub repository" }),
    ).toHaveValue(repository.full_name);
    expect(
      screen.getByRole("textbox", { name: "GitHub repository" }),
    ).toHaveAttribute("readonly");
    await screen.findByRole("combobox", { name: "Eval to run" });
    fireEvent.change(screen.getByRole("textbox", { name: "Lens URL" }), {
      target: { value: "https://lens.example.test" },
    });
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(
      screen.getByRole("tabpanel", { name: ".github/workflows/lens.yml" }),
    ).toHaveTextContent("report-via-app: true");
    expect(screen.getByRole("tabpanel")).not.toHaveTextContent(
      "pull-requests: write",
    );
    expect(screen.getByRole("tabpanel")).not.toHaveTextContent("checks: write");
  });
});
