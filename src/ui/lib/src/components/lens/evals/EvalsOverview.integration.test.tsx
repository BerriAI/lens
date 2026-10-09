import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  renderWithLens,
  stubGateway,
  type GatewayRequest,
} from "../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../tests/test-utils";
import { EvalsView } from "./EvalsView";
import { evalRun } from "./runs/testRuns";
import type { EvalDefinition, EvalRun } from "./runs/types";

vi.mock("../../../lib/http/requests", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../lib/http/requests")>()),
  proxyBaseUrl: "",
  getProxyBaseUrl: () => "",
}));

const definition: EvalDefinition = {
  name: "support-regressions",
  spec: {
    agent: "support-agent",
    dataset_id: "dataset-1",
    revision: 3,
    scorers: [{ kind: "task_completed" }],
    trials: 1,
    baseline: "main",
    gate: { regressions: 0 },
    timeout_per_trial_ms: 60_000,
  },
  updated_at: "2026-10-01T00:00:00Z",
};
const ciUrl = "https://github.com/test-owner/agent/actions/runs/1234";
const completed = evalRun("latest-run", {
  eval: definition.name,
  agent: definition.spec.agent,
  ci_url: ciUrl,
});
const connection = {
  configured: true,
  app_slug: "lens-app",
  connection: {
    agent: definition.spec.agent,
    repository_id: 42,
    repository: "test-owner/agent",
    installation_id: 1,
    default_branch: "main",
    connected_at: "2026-10-01T00:00:00Z",
    available: true,
  },
};
let proxy = stubGateway();
let runs: readonly EvalRun[];
const serve = (path: string, request: GatewayRequest) => {
  if (path === "/lens/evals") return [definition];
  if (path === `/lens/evals/${definition.name}`) return definition;
  if (path === "/lens/datasets")
    return [
      {
        id: "dataset-1",
        name: "Support cases",
        revision: 3,
        case_count: 4,
        agent_name: definition.spec.agent,
      },
    ];
  if (path === "/lens/evals/runs") return runs;
  if (path === "/lens/github/status") return connection;
  throw new Error(
    `unexpected request ${path} ${JSON.stringify(request.query)}`,
  );
};
beforeEach(() => {
  testQueryClient.clear();
  proxy = stubGateway();
  runs = [completed];
  proxy.get.mockImplementation(serve);
});

describe("Eval execution overview", () => {
  it("shows code execution separately from the dataset without claiming the eval is Python", async () => {
    const user = userEvent.setup();
    renderWithLens(<EvalsView />, { searchParams: "?tab=evals" });
    const row = await screen.findByRole("row", { name: definition.name });
    expect(within(row).getByText("Code / SDK")).toBeVisible();
    expect(within(row).getByText("Support cases@3")).toBeVisible();
    expect(screen.queryByText("REGRESSION LAB")).not.toBeInTheDocument();
    await user.click(row);
    expect(await screen.findByText("Runs in your code")).toBeVisible();
    expect(
      await screen.findByRole("button", { name: "Run again" }),
    ).toBeEnabled();
    expect(screen.getByText(/Reruns the workflow for/)).toHaveTextContent(
      "The workflow controls the dataset revision and may run other evals",
    );
  });

  it("requests a real rerun and waits for a new result instead of treating the old pass as success", async () => {
    const user = userEvent.setup();
    proxy.post.mockResolvedValue({
      ci_url: ciUrl,
      requested_at: "2026-10-09T00:00:00Z",
    });
    renderWithLens(<EvalsView />, {
      searchParams: `?tab=evals&eval=${definition.name}`,
    });
    await user.click(await screen.findByRole("button", { name: "Run again" }));
    await waitFor(() =>
      expect(proxy.post).toHaveBeenCalledWith(
        "/lens/github/rerun",
        expect.objectContaining({ body: { run_id: completed.id } }),
      ),
    );
    expect(
      await screen.findByRole("button", { name: "Run requested" }),
    ).toBeDisabled();
    expect(
      screen.getByText(/Waiting for this workflow to send results/),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "Open workflow" })).toHaveAttribute(
      "href",
      ciUrl,
    );
    expect(proxy.put).not.toHaveBeenCalled();
    runs = [evalRun("new-run", { ...completed, id: "new-run" }), completed];
    await testQueryClient.invalidateQueries();
    expect(await screen.findByText(/New results received/)).toBeVisible();
    expect(screen.getByRole("button", { name: "Run again" })).toBeEnabled();
  });

  it("shows a rejected rerun as an error and leaves retry available", async () => {
    const user = userEvent.setup();
    proxy.post.mockRejectedValue(
      new Error("GitHub App needs Actions write permission"),
    );
    renderWithLens(<EvalsView />, {
      searchParams: `?tab=evals&eval=${definition.name}`,
    });
    await user.click(await screen.findByRole("button", { name: "Run again" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not start the run. GitHub App needs Actions write permission",
    );
    expect(screen.getByRole("button", { name: "Run again" })).toBeEnabled();
    expect(
      screen.queryByRole("button", { name: "Run requested" }),
    ).not.toBeInTheDocument();
  });

  it("explains how to run a new code eval with its exact saved name", async () => {
    const user = userEvent.setup();
    runs = [];
    renderWithLens(<EvalsView />, {
      searchParams: `?tab=evals&eval=${definition.name}`,
    });
    const project = await screen.findByRole("region", {
      name: "Run in your project",
    });
    expect(screen.getByText(/Saving it does not run your agent/)).toBeVisible();
    expect(
      within(project).getByText("python -m pytest tests/test_lens_eval.py -v"),
    ).toBeVisible();
    await user.click(
      within(project).getByText("Python test: tests/test_lens_eval.py"),
    );
    expect(
      within(project).getByText(
        new RegExp(`lens.evals.test\\("${definition.name}"\\)`),
      ),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "Run again" }),
    ).not.toBeInTheDocument();
    expect(proxy.post).not.toHaveBeenCalled();
  });

  it("identifies a saved HTTP contract using the v2 definition instead of advertising a Python implementation", async () => {
    const httpDefinition = {
      ...definition,
      spec: {
        ...definition.spec,
        agent_io: { version: 1, connection: "agent-service" },
      },
    };
    proxy.get.mockImplementation((path: string, request: GatewayRequest) =>
      path === "/lens/evals" ? [httpDefinition] : serve(path, request),
    );
    renderWithLens(<EvalsView />, { searchParams: "?tab=evals" });
    const row = await screen.findByRole("row", { name: definition.name });
    expect(within(row).getByText("HTTP contract")).toBeVisible();
    expect(within(row).queryByText("Code / SDK")).not.toBeInTheDocument();
  });
});
