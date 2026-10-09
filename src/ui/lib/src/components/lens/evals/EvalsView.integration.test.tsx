import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderWithProviders, testQueryClient } from "../../../../tests/test-utils";
import { renderWithLens, stubGateway, type GatewayRequest } from "../../../../tests/lens-test-utils";
import { LensServicesProvider } from "../data/LensServices";
import { createLensDemo } from "../data/demo/createLensDemo";
import type { Dataset, DatasetSummary } from "../datasets/types";
import { EvalsView } from "./EvalsView";
import { RunsTab } from "./runs/RunsTab";
import { diff, evalRun, runCase, step, summary, trial } from "./runs/testRuns";
import type { EvalDefinition, EvalRun, RunCase } from "./runs/types";

vi.mock("../../../lib/http/requests", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../lib/http/requests")>()),
  proxyBaseUrl: "",
  getProxyBaseUrl: () => "",
}));

const dataset: Dataset = {
  id: "ds-1",
  name: "Refund agent regressions",
  agent_name: "refund-agent",
  team_id: "",
  created_at: "2026-10-01T10:00:00Z",
  revision: 2,
  created_by: "admin",
  cases: [],
};

const datasetSummary: DatasetSummary = {
  id: dataset.id,
  name: dataset.name,
  agent_name: dataset.agent_name,
  revision: dataset.revision,
  case_count: 0,
  updated_at: "2026-10-02T10:00:00Z",
};

const definition: EvalDefinition = {
  name: "agent-regressions",
  spec: {
    agent: "refund-agent",
    dataset_id: dataset.id,
    revision: null,
    scorers: [
      { kind: "task_completed" },
      { kind: "called_before", first: "check_refund_policy", then: "issue_refund" },
    ],
    trials: 3,
    baseline: "main",
    gate: { regressions: 0, critical: 0, pass_rate: null, cost_per_case: null, min: {} },
    timeout_per_trial_ms: 1_200_000,
  },
  updated_at: "2026-10-08T10:00:00Z",
};

const regression = diff("case-refund", {
  title: "Can I get a refund for order 42?",
  critical: true,
});

const mainRun = evalRun("run-main", {
  version: "1111111aaaa",
  summary: summary({
    passed: 4,
    baseline_run_id: null,
    baseline_version: null,
  }),
});

const redRun = evalRun("run-red", {
  branch: "drop-policy-check",
  version: "2222222bbbb",
  pr: 7,
  summary: summary({
    regressions: [regression],
    gate: {
      passed: false,
      reasons: ["1 critical case regressed against main"],
    },
  }),
});

const erroredRun = evalRun("run-error", {
  branch: "flaky-ci",
  version: "3333333cccc",
  pr: 8,
  status: "failed",
  summary: null,
  failure: "dataset revision 9 not found",
});

const failedCheck = { scorer: "called_before", passed: false };
const cases: Record<string, RunCase> = {
  "run-main": runCase(true, [
    trial(1, [step("lookup_order", 0), step("check_refund_policy", 10_000_000), step("issue_refund", 20_000_000)], {
      checks: [{ ...failedCheck, passed: true }],
    }),
  ]),
  "run-red": runCase(false, [
    trial(1, [step("lookup_order", 0), step("issue_refund", 20_000_000)], {
      checks: [failedCheck],
    }),
  ]),
};

let proxy = stubGateway();

const serve = (path: string, request: GatewayRequest) => {
  if (path === "/lens/datasets") return [datasetSummary];
  if (path === `/lens/datasets/${dataset.id}`) return dataset;
  if (path === "/lens/evals") return [definition];
  if (path === `/lens/evals/${definition.name}`) return definition;
  if (path === "/lens/evals/runs" && request.query.eval === definition.name) return [redRun, erroredRun, mainRun];
  const run = [mainRun, redRun, erroredRun].find((item) => path === `/lens/evals/runs/${item.id}`);
  if (run) return run;
  const listedRunId = path.match(/^\/lens\/evals\/runs\/([^/]+)\/cases$/)?.[1];
  if (listedRunId)
    return cases[listedRunId]
      ? [
          { ...cases[listedRunId], case_id: regression.case_id, title: regression.title },
          { case_id: "unchanged-pass", title: "Keeps existing files", critical: false, passed: true },
        ]
      : [];
  if (path === "/lens/github/status") return { configured: false, app_slug: null, connection: null };
  const runId = path.match(/^\/lens\/evals\/runs\/([^/]+)\/cases\/case-refund$/)?.[1];
  if (runId && cases[runId]) return cases[runId];
  throw new Error(`unexpected GET ${path} ${JSON.stringify(request.query)}`);
};

beforeEach(() => {
  testQueryClient.clear();
  proxy = stubGateway();
  proxy.get.mockImplementation(serve);
});

async function expectCaseResult() {
  const result = await screen.findByRole("region", { name: "Case result" });
  expect(result).toHaveTextContent(regression.title);
  expect(await within(result).findByText("Trace unavailable")).toBeInTheDocument();
  expect(within(result).getByRole("list", { name: "Checks" })).toHaveTextContent("Failedcalled_before");
  expect(screen.queryByRole("region", { name: "Case comparison" })).not.toBeInTheDocument();
}

const evalPage = `?tab=evals&eval=${definition.name}`;

describe("Evals", () => {
  it("should list evals and all cases, then open a case result", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const first = renderWithLens(<EvalsView />, { searchParams: "?tab=evals", onUrlUpdate });

    const row = await screen.findByRole("row", { name: definition.name });
    expect(row).toHaveTextContent("Refund agent regressions@2");
    expect(row).toHaveTextContent("Code");

    await user.click(row);
    await user.click(await screen.findByRole("row", { name: "drop-policy-check@2222222" }));
    const caseList = await screen.findByRole("navigation", { name: "Test cases" });
    expect(within(caseList).getByRole("button", { name: /Keeps existing files/ })).toHaveTextContent("Passed");
    expect(screen.getByLabelText("Case totals")).toHaveTextContent("3 passed1 failed4 cases");
    await user.click(within(caseList).getByRole("button", { name: /Can I get a refund/ }));
    await expectCaseResult();
    const url = onUrlUpdate.mock.lastCall?.[0].queryString as string;
    expect(Object.fromEntries(new URLSearchParams(url))).toMatchObject({
      tab: "evals",
      eval: definition.name,
      eval_run: redRun.id,
      eval_case: regression.case_id,
    });
    first.unmount();
    testQueryClient.clear();

    renderWithLens(<EvalsView />, { searchParams: url });
    await expectCaseResult();
  });

  it("should open a case from a PR comment link without listing runs", async () => {
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${redRun.id}&eval_case=${regression.case_id}`,
    });

    await expectCaseResult();
    expect(proxy.get).not.toHaveBeenCalledWith("/lens/evals/runs", expect.anything());
  });

  it("should show why an errored run failed", async () => {
    const user = userEvent.setup();
    renderWithLens(<EvalsView />, { searchParams: evalPage });

    const row = await screen.findByRole("row", { name: "flaky-ci@3333333" });
    expect(within(row).getByText("Error")).toBeInTheDocument();

    await user.click(row);
    expect(await screen.findByRole("alert")).toHaveTextContent("dataset revision 9 not found");
    expect(screen.queryByText(/scoring/)).not.toBeInTheDocument();
  });

  it("should show a missing case honestly and return to all cases", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${redRun.id}&eval_case=case-gone`,
      onUrlUpdate,
    });

    const missing = await screen.findByRole("alert", { name: "Case unavailable" });
    expect(missing).toHaveTextContent("Case case-gone was not found in this run");

    await user.click(within(missing).getByRole("button", { name: "All cases" }));
    expect(await screen.findByRole("button", { name: /Keeps existing files/ })).toBeInTheDocument();
    expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has("eval_case")).toBe(false);
  });

  it("creates an eval from the form, stores it in Lens, and opens it", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const created: EvalDefinition = { ...definition, name: "refund-order" };
    proxy.get.mockImplementation((path: string, request: GatewayRequest) =>
      path === "/lens/evals/refund-order" ? created : serve(path, request),
    );
    proxy.put.mockResolvedValue(created);
    renderWithLens(<EvalsView />, { searchParams: "?tab=evals", onUrlUpdate });

    await user.click(await screen.findByRole("button", { name: "New eval" }));
    const form = screen.getByRole("form", { name: "New eval" });
    act(() =>
      fireEvent.change(within(form).getByRole("textbox", { name: "Name" }), { target: { value: "refund-order" } }),
    );
    await user.click(within(form).getByRole("combobox", { name: "Dataset" }));
    await user.click(await screen.findByRole("option", { name: "Refund agent regressions@2" }));
    expect(within(form).getByRole("textbox", { name: "Agent" })).toHaveValue("refund-agent");
    await user.click(within(form).getByRole("checkbox", { name: "Tool order" }));
    act(() =>
      fireEvent.change(within(form).getByRole("textbox", { name: "First tool" }), {
        target: { value: "check_refund_policy" },
      }),
    );
    act(() =>
      fireEvent.change(within(form).getByRole("textbox", { name: "Then tool" }), { target: { value: "issue_refund" } }),
    );
    await user.click(within(form).getByRole("button", { name: "Create eval" }));

    await waitFor(() => expect(proxy.put).toHaveBeenCalledOnce());
    expect(proxy.put.mock.lastCall?.[0]).toBe("/lens/evals/refund-order");
    expect(proxy.put.mock.lastCall?.[1].body).toEqual({
      agent: "refund-agent",
      dataset_id: dataset.id,
      scorers: [
        { kind: "task_completed" },
        { kind: "called_before", first: "check_refund_policy", then: "issue_refund" },
      ],
      trials: 3,
      baseline: "main",
      gate: { regressions: 0, critical: 0, pass_rate: null, cost_per_case: null, min: {} },
    });
    expect(await screen.findByRole("heading", { name: "refund-order" })).toBeInTheDocument();
    expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("eval")).toBe("refund-order");
  });

  it("opens GitHub setup for an eval with no runs, then opens the first run once it arrives", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    let runs: EvalRun[] = [];
    proxy.get.mockImplementation((path: string, request: GatewayRequest) => {
      if (path === "/lens/evals/runs") return runs;
      if (path === "/lens/github/status") return { configured: false, app_slug: null, connection: null };
      return serve(path, request);
    });
    renderWithLens(<EvalsView />, { searchParams: evalPage, onUrlUpdate });

    const connect = await screen.findByRole("region", {
      name: "Connect agent",
    });
    expect(within(connect).getByRole("button", { name: "Connect GitHub" })).toBeEnabled();
    expect(within(connect).queryByRole("tabpanel")).not.toBeInTheDocument();
    await user.click(within(connect).getByRole("button", { name: "Connect GitHub" }));
    const dialog = await screen.findByRole("dialog", {
      name: "Connect GitHub",
    });
    expect(await within(dialog).findByRole("heading", { name: "GitHub connection unavailable" })).toBeVisible();
    expect(within(dialog).queryByRole("textbox", { name: "GitHub repository" })).not.toBeInTheDocument();
    await user.keyboard("{Escape}");

    runs = [redRun];
    await testQueryClient.invalidateQueries();

    expect(await screen.findByRole("button", { name: /Keeps existing files/ })).toBeInTheDocument();
    expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("eval_run")).toBe(redRun.id);
  });

  it("serves an empty run list in sample mode without calling the proxy", async () => {
    renderWithProviders(
      <LensServicesProvider services={createLensDemo()}>
        <RunsTab definition={definition} datasetName={dataset.name} revision={dataset.revision} onOpen={vi.fn()} />
      </LensServicesProvider>,
    );

    expect(await screen.findByRole("region", { name: "Connect agent" })).toBeInTheDocument();
    expect(proxy.get).not.toHaveBeenCalled();
  });
});
