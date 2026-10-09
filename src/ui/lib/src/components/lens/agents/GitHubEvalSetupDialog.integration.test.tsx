import { fireEvent, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithLens, stubGateway } from "../../../../tests/lens-test-utils";
import {
  chooseSelectOption,
  testQueryClient,
} from "../../../../tests/test-utils";
import type { DatasetSummary } from "../datasets/types";
import { evalRun, summary } from "../evals/runs/testRuns";
import type { EvalDefinition, EvalRun } from "../evals/runs/types";
import { GitHubEvalSetupDialog } from "./GitHubEvalSetupDialog";

const agent = "qa-agent";
const repository = "lens-test/agent";
const repositoryUrl = `https://github.com/${repository}`;
const dataset: DatasetSummary = {
  id: "qa-dataset",
  name: "Regression cases",
  agent_name: agent,
  revision: 2,
  case_count: 4,
  updated_at: "2026-10-01T00:00:00Z",
};
const definition: EvalDefinition = {
  name: "agent-regressions",
  spec: {
    agent,
    dataset_id: dataset.id,
    revision: null,
    scorers: [{ kind: "task_completed" }],
    trials: 3,
    baseline: "main",
    gate: {
      regressions: 0,
      critical: 0,
      pass_rate: null,
      cost_per_case: null,
      min: {},
    },
    timeout_per_trial_ms: 1_200_000,
  },
  updated_at: "2026-10-01T00:00:00Z",
};
const otherEval: EvalDefinition = { ...definition, name: "agent-smoke" };
const otherAgent: EvalDefinition = {
  ...definition,
  name: "unrelated-agent-eval",
  spec: { ...definition.spec, agent: "another-agent" },
};
const run = (id: string, overrides: Partial<EvalRun> = {}) =>
  evalRun(id, {
    agent,
    eval: definition.name,
    ci_url: `${repositoryUrl}/actions/runs/123`,
    ...overrides,
  });

let gateway = stubGateway();
const serve =
  (runs: readonly EvalRun[] = []) =>
  (path: string) => {
    if (path === "/lens/evals") return [definition, otherEval, otherAgent];
    if (path === "/lens/datasets") return [dataset];
    if (path === "/lens/evals/runs") return runs;
    throw new Error(`Unexpected GET ${path}`);
  };
const change = (name: string, value: string) =>
  fireEvent.change(screen.getByRole("textbox", { name }), {
    target: { value },
  });
const prepare = async (user: ReturnType<typeof userEvent.setup>) => {
  await screen.findByRole("combobox", { name: "Eval to run" });
  change("GitHub repository", repository);
  change("Lens URL", "https://lens.example.test");
  await user.click(screen.getByRole("button", { name: "Continue" }));
};
const verify = async (user: ReturnType<typeof userEvent.setup>) => {
  await prepare(user);
  await user.click(
    screen.getByRole("button", { name: "I’ve added the workflow" }),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Check for PR eval" }),
    ).toBeEnabled(),
  );
};

beforeEach(() => {
  testQueryClient.clear();
  gateway = stubGateway();
  gateway.get.mockImplementation(serve());
});

describe("GitHub eval workflow setup", () => {
  it("chooses this agent’s eval and carries repository, runtime and dataset into reviewable setup files", async () => {
    const user = userEvent.setup();
    renderWithLens(
      <GitHubEvalSetupDialog agent={agent} onOpenChange={vi.fn()} />,
    );
    const selector = await screen.findByRole("combobox", {
      name: "Eval to run",
    });
    await user.click(selector);
    expect(
      screen.queryByRole("option", { name: otherAgent.name }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: otherEval.name }));
    change("GitHub repository", `${repositoryUrl}.git`);
    change("Lens URL", "https://lens.example.test/");
    await user.click(screen.getByText("Agent runtime"));
    change("Existing eval task (optional)", "my_agent.evals:task");
    change("Install and start your agent (optional)", "npm ci");
    await user.click(screen.getByRole("button", { name: "Continue" }));

    const workflow = screen.getByRole("tabpanel", {
      name: ".github/workflows/lens.yml",
    });
    expect(workflow).toHaveTextContent("npm ci");
    expect(workflow).toHaveTextContent("evals/agent_smoke.py");
    expect(workflow).toHaveTextContent(
      "github.event.pull_request.head.repo.full_name == github.repository",
    );
    expect(workflow).toHaveTextContent("${{ secrets.LENS_API_KEY }}");
    expect(workflow).toHaveTextContent("${{ vars.LENS_BASE_URL }}");
    expect(
      screen.getByRole("link", { name: "Open repository secrets" }),
    ).toHaveAttribute("href", `${repositoryUrl}/settings/secrets/actions`);
    expect(
      screen.getByRole("link", { name: "Open repository variables" }),
    ).toHaveAttribute("href", `${repositoryUrl}/settings/variables/actions`);
    expect(screen.getByText(/A tracing key cannot run evals/)).toBeVisible();
    await user.click(screen.getByRole("tab", { name: "evals/agent_smoke.py" }));
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      "from my_agent.evals import task as agent_task",
    );
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      'data="Regression cases@2"',
    );
    await user.click(screen.getByRole("tab", { name: "pyproject.toml" }));
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      `project = "${agent}"`,
    );
    expect(gateway.post).not.toHaveBeenCalled();
    expect(gateway.put).not.toHaveBeenCalled();
  });

  it("rejects a localhost Lens URL and explains that an unconfigured adapter must be implemented", async () => {
    const user = userEvent.setup();
    renderWithLens(
      <GitHubEvalSetupDialog agent={agent} onOpenChange={vi.fn()} />,
    );
    await screen.findByRole("combobox", { name: "Eval to run" });
    change("GitHub repository", repository);
    change("Lens URL", "https://localhost:3100");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "GitHub-hosted runners cannot reach localhost",
    );
    expect(screen.queryByRole("tabpanel")).not.toBeInTheDocument();
    change("Lens URL", "https://lens.example.test");
    await user.click(screen.getByRole("button", { name: "Continue" }));
    expect(
      screen.getByText(/The template will fail until you connect your agent/),
    ).toBeVisible();
    await user.click(
      screen.getByRole("tab", { name: "evals/agent_regressions.py" }),
    );
    expect(screen.getByRole("tabpanel")).toHaveTextContent(
      "raise NotImplementedError",
    );
  });

  it("does not verify unrelated repositories, agents, evals, or local runs", async () => {
    const user = userEvent.setup();
    gateway.get.mockImplementation(
      serve([
        run("wrong-repo", {
          pr: 11,
          ci_url: "https://github.com/lens-test/other/actions/runs/123",
        }),
        run("wrong-agent", { pr: 12, agent: "another-agent" }),
        run("wrong-eval", { pr: 13, eval: otherEval.name }),
        run("local", { pr: 14, ci_url: null }),
      ]),
    );
    renderWithLens(
      <GitHubEvalSetupDialog agent={agent} onOpenChange={vi.fn()} />,
    );
    await verify(user);
    expect(
      screen.getByRole("heading", { name: "Waiting for your first PR eval" }),
    ).toBeVisible();
    expect(
      screen.queryByRole("button", { name: "View eval result" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("link", { name: /Open PR/ }),
    ).not.toBeInTheDocument();
    expect(gateway.get).toHaveBeenCalledWith(
      "/lens/evals/runs",
      expect.objectContaining({
        query: {
          agent,
          eval: definition.name,
          include_ci: "true",
          limit: "100",
        },
      }),
    );
  });

  it("distinguishes a running PR, a failure, and a completed result without claiming its comment was published", async () => {
    const user = userEvent.setup();
    const onOpenChange = vi.fn();
    const onUrlUpdate = vi.fn();
    const baseline = run("baseline");
    const pull = run("candidate", {
      pr: 17,
      branch: "feature",
      status: "scoring",
      summary: null,
    });
    gateway.get.mockImplementation(serve([pull, baseline]));
    renderWithLens(
      <GitHubEvalSetupDialog agent={agent} onOpenChange={onOpenChange} />,
      { onUrlUpdate },
    );
    await verify(user);
    expect(
      screen.getByText(
        "PR #17 is scoring. Check again when the workflow finishes",
      ),
    ).toBeVisible();
    expect(screen.getByText("Baseline received from main")).toBeVisible();

    gateway.get.mockImplementation(
      serve([
        { ...pull, status: "failed", failure: "Agent could not start" },
        baseline,
      ]),
    );
    await user.click(screen.getByRole("button", { name: "Check for PR eval" }));
    expect(
      await screen.findByRole("heading", { name: "PR eval needs attention" }),
    ).toBeVisible();
    expect(screen.getByText("Agent could not start")).toBeVisible();

    gateway.get.mockImplementation(
      serve([
        {
          ...pull,
          status: "done",
          summary: summary({
            gate: { passed: false, reasons: ["regression"] },
          }),
        },
        baseline,
      ]),
    );
    await user.click(screen.getByRole("button", { name: "Check for PR eval" }));
    expect(
      await screen.findByRole("heading", { name: "PR eval received" }),
    ).toBeVisible();
    expect(
      screen.getByText(/Gate failed, review the result before merging/),
    ).toBeVisible();
    expect(
      screen.getByText(/Lens receives the eval before GitHub publishes it/),
    ).toBeVisible();
    expect(screen.getByRole("link", { name: "Open PR #17" })).toHaveAttribute(
      "href",
      `${repositoryUrl}/pull/17`,
    );
    await user.click(screen.getByRole("button", { name: "View eval result" }));
    await waitFor(() =>
      expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("eval_run")).toBe(
        pull.id,
      ),
    );
    expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("agent")).toBe(
      agent,
    );
    expect(onUrlUpdate.mock.lastCall?.[0].searchParams.get("eval")).toBe(
      definition.name,
    );
    expect(onOpenChange).toHaveBeenCalledWith(false);
  });

  it("recovers when checking PR runs fails", async () => {
    const user = userEvent.setup();
    renderWithLens(
      <GitHubEvalSetupDialog agent={agent} onOpenChange={vi.fn()} />,
    );
    await prepare(user);
    gateway.get.mockImplementation((path) => {
      if (path === "/lens/evals/runs") throw new Error("Unavailable");
      return serve()(path);
    });
    await user.click(
      screen.getByRole("button", { name: "I’ve added the workflow" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Could not check eval runs",
    );
    gateway.get.mockImplementation(serve([run("received", { pr: 19 })]));
    await user.click(screen.getByRole("button", { name: "Check for PR eval" }));
    expect(
      await screen.findByRole("heading", { name: "PR eval received" }),
    ).toBeVisible();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("routes an agent without datasets to the prerequisite instead of promising a connected workflow", async () => {
    const user = userEvent.setup();
    const onOpenDatasets = vi.fn();
    gateway.get.mockReturnValue([]);
    renderWithLens(
      <GitHubEvalSetupDialog
        agent={agent}
        onOpenChange={vi.fn()}
        onOpenDatasets={onOpenDatasets}
      />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Create a dataset first" }),
    );
    expect(onOpenDatasets).toHaveBeenCalledOnce();
    expect(screen.getByRole("button", { name: "Continue" })).toBeDisabled();
  });

  it("prefills the current agent while creating its first eval and returns it to GitHub setup", async () => {
    const user = userEvent.setup();
    gateway.get.mockImplementation((path) =>
      path === "/lens/evals" ? [] : serve()(path),
    );
    gateway.put.mockReturnValue(definition);
    renderWithLens(
      <GitHubEvalSetupDialog agent={agent} onOpenChange={vi.fn()} />,
    );
    await user.click(
      await screen.findByRole("button", { name: "Create eval" }),
    );
    expect(screen.getByRole("textbox", { name: "Agent" })).toHaveValue(agent);
    change("Name", definition.name);
    await chooseSelectOption(
      user,
      screen.getByRole("combobox", { name: "Dataset" }),
      /Regression cases/,
    );
    gateway.get.mockImplementation(serve());
    await user.click(screen.getByRole("button", { name: "Create eval" }));
    expect(
      await screen.findByRole("combobox", { name: "Eval to run" }),
    ).toHaveTextContent(definition.name);
    expect(gateway.put).toHaveBeenCalledWith(
      `/lens/evals/${definition.name}`,
      expect.objectContaining({
        body: expect.objectContaining({ agent, dataset_id: dataset.id }),
      }),
    );
  });
});
