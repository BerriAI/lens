import { fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../../tests/test-utils";
import {
  renderWithLens,
  stubGateway,
  type GatewayRequest,
} from "../../../../tests/lens-test-utils";
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
    trial(
      1,
      [
        step("lookup_order", 0),
        step("check_refund_policy", 10_000_000),
        step("issue_refund", 20_000_000),
      ],
      {
        checks: [{ ...failedCheck, passed: true }],
      },
    ),
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
  if (path === "/lens/evals/runs" && request.query.eval === definition.name)
    return [redRun, erroredRun, mainRun];
  const run = [mainRun, redRun, erroredRun].find(
    (item) => path === `/lens/evals/runs/${item.id}`,
  );
  if (run) return run;
  const runId = path.match(
    /^\/lens\/evals\/runs\/([^/]+)\/cases\/case-refund$/,
  )?.[1];
  if (runId && cases[runId]) return cases[runId];
  throw new Error(`unexpected GET ${path} ${JSON.stringify(request.query)}`);
};

beforeEach(() => {
  testQueryClient.clear();
  proxy = stubGateway();
  proxy.get.mockImplementation(serve);
});

const toolNames = (trace: string) =>
  within(
    within(screen.getByRole("region", { name: `${trace} trace` })).getByRole(
      "list",
      { name: "Tool calls" },
    ),
  )
    .getAllByRole("listitem")
    .map((item) => item.getAttribute("aria-label"));

async function expectComparison() {
  await waitFor(() =>
    expect(screen.getByLabelText("Diagnosis")).toHaveTextContent(
      "regressed: never called check_refund_policy, which main called · called_before failed 1/1 trials",
    ),
  );
  expect(
    await screen.findAllByRole("list", { name: "Tool calls" }),
  ).toHaveLength(2);
  expect(toolNames("main")).toEqual([
    "lookup_order",
    "check_refund_policy (missing in candidate)",
    "issue_refund",
  ]);
  expect(toolNames("candidate")).toEqual(["lookup_order", "issue_refund"]);
  expect(
    screen.getByRole("region", { name: "Case comparison" }),
  ).toHaveTextContent(regression.title);
}

const evalPage = `?tab=evals&eval=${definition.name}`;

describe("Evals", () => {
  it("lists evals, opens one onto its runs, and lands a red run on its first regression", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const first = renderWithLens(<EvalsView />, { searchParams: "?tab=evals", onUrlUpdate });

    const row = await screen.findByRole("row", { name: definition.name });
    expect(row).toHaveTextContent("Refund agent regressions@2");
    expect(row).toHaveTextContent("check_refund_policy before issue_refund");
    expect(row).toHaveTextContent("regressions ≤ 0 · critical ≤ 0");

    await user.click(row);
    await user.click(await screen.findByRole("row", { name: "drop-policy-check@2222222" }));
    expect(await screen.findByRole("list", { name: "Gate reasons" })).toHaveTextContent(
      "1 critical case regressed against main",
    );
    expect(
      within(screen.getByRole("navigation", { name: "Changed cases" })).getByRole("button", { current: true }),
    ).toHaveTextContent(regression.title);
    await expectComparison();

    await user.click(screen.getByRole("button", { name: /Can I get a refund/ }));
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
    await expectComparison();
  });

  it("lands on the regressed case from a PR comment link without listing runs", async () => {
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${redRun.id}&eval_case=${regression.case_id}`,
    });

    await expectComparison();
    expect(proxy.get).not.toHaveBeenCalledWith("/lens/evals/runs", expect.anything());
  });

  it("shows why an errored run failed instead of saying it is still scoring", async () => {
    const user = userEvent.setup();
    renderWithLens(<EvalsView />, { searchParams: evalPage });

    const row = await screen.findByRole("row", { name: "flaky-ci@3333333" });
    expect(within(row).getByText("gate errored")).toBeInTheDocument();

    await user.click(row);
    expect(await screen.findByRole("alert")).toHaveTextContent("error: dataset revision 9 not found");
    expect(screen.queryByText(/scoring/)).not.toBeInTheDocument();
  });

  it("says so when a PR link names a case that did not change, and clears it on dismiss", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${redRun.id}&eval_case=case-gone`,
      onUrlUpdate,
    });

    const missing = await screen.findByRole("alert");
    expect(missing).toHaveTextContent("Case case-gone did not regress or get fixed in this run.");

    await user.click(within(missing).getByRole("button", { name: "Dismiss" }));
    await expectComparison();
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
    fireEvent.change(within(form).getByRole("textbox", { name: "Name" }), { target: { value: "refund-order" } });
    await user.click(within(form).getByRole("combobox", { name: "Dataset" }));
    await user.click(await screen.findByRole("option", { name: "Refund agent regressions@2" }));
    expect(within(form).getByRole("textbox", { name: "Agent" })).toHaveValue("refund-agent");
    await user.click(within(form).getByRole("checkbox", { name: "Tool order" }));
    fireEvent.change(within(form).getByRole("textbox", { name: "First tool" }), {
      target: { value: "check_refund_policy" },
    });
    fireEvent.change(within(form).getByRole("textbox", { name: "Then tool" }), { target: { value: "issue_refund" } });
    await user.click(within(form).getByRole("button", { name: "Create eval" }));

    await waitFor(() => expect(proxy.put).toHaveBeenCalledOnce());
    expect(proxy.put.mock.lastCall?.[0]).toBe("/lens/evals/refund-order");
    expect(proxy.put.mock.lastCall?.[1].body).toEqual({
      agent: "refund-agent",
      dataset_id: dataset.id,
      scorers: [{ kind: "task_completed" }, { kind: "called_before", first: "check_refund_policy", then: "issue_refund" }],
      trials: 3,
      baseline: "main",
      gate: { regressions: 0, critical: 0, pass_rate: null, cost_per_case: null, min: {} },
    });
    expect(await screen.findByRole("heading", { name: "refund-order" })).toBeInTheDocument();
    expect(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get("eval")).toBe("refund-order");
  });

  it("offers Connect to GitHub for an eval with no runs, then opens the first run once it arrives", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    let runs: EvalRun[] = [];
    proxy.get.mockImplementation((path: string, request: GatewayRequest) =>
      path === "/lens/evals/runs" ? runs : serve(path, request),
    );
    renderWithLens(<EvalsView />, { searchParams: evalPage, onUrlUpdate });

    const connect = await screen.findByRole("region", { name: "Connect agent" });
    expect(within(connect).getByRole("button", { name: /Connect to GitHub/ })).toBeInTheDocument();
    expect(within(connect).queryByRole("tabpanel")).not.toBeInTheDocument();
    await user.click(within(connect).getByRole("button", { name: "Set up manually" }));
    await user.click(within(connect).getByRole("tab", { name: "evals/agent_regressions.py" }));
    expect(within(connect).getByRole("tabpanel", { name: "evals/agent_regressions.py" })).toHaveTextContent(
      'scorers.called_before("check_refund_policy", "issue_refund")',
    );

    runs = [redRun];
    await testQueryClient.invalidateQueries();

    await expectComparison();
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
