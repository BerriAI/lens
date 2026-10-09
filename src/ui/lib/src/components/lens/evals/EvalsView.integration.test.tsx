import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
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
import researchTrace from "../traces/__fixtures__/research_trace.json";
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
      {
        kind: "called_before",
        first: "check_refund_policy",
        then: "issue_refund",
      },
    ],
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
        traces: [{ trace_id: "before-trace", trace_ref: "before-owner" }],
      },
    ),
  ]),
  "run-red": runCase(false, [
    trial(1, [step("lookup_order", 0), step("issue_refund", 20_000_000)], {
      checks: [failedCheck],
      traces: [{ trace_id: "after-trace", trace_ref: "after-owner" }],
    }),
  ]),
};

let proxy = stubGateway();

const serve = (path: string, request: GatewayRequest) => {
  if (path.startsWith("/v1/traces/"))
    return {
      ...researchTrace,
      summary: {
        ...researchTrace.summary,
        trace_id: path.split("/").at(-1),
        trace_ref: request.query.trace_ref,
      },
    };
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
  if (/^\/lens\/evals\/runs\/[^/]+\/cases$/.test(path))
    return [
      {
        case_id: regression.case_id,
        title: regression.title,
        critical: regression.critical,
        passed: path.includes("run-main"),
      },
    ];
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
    const first = renderWithLens(<EvalsView />, {
      searchParams: "?tab=evals&trace=old-trace&trace_ref=old-owner&fullscreen=true",
      onUrlUpdate,
    });

    const row = await screen.findByRole("row", { name: definition.name });
    expect(row).toHaveTextContent("Refund agent regressions@2");
    expect(row).toHaveTextContent("check_refund_policy before issue_refund");
    expect(row).toHaveTextContent("regressions ≤ 0 · critical ≤ 0");

    await user.click(row);
    await waitFor(() =>
      expect(Object.fromEntries(onUrlUpdate.mock.lastCall?.[0].searchParams)).toEqual({
        tab: "evals",
        eval: definition.name,
      }),
    );
    await user.click(
      await screen.findByRole("row", { name: "drop-policy-check@2222222" }),
    );
    expect(
      await screen.findByRole("list", { name: "Gate reasons" }),
    ).toHaveTextContent("1 critical case regressed against main");
    expect(
      within(screen.getByRole("navigation", { name: "Cases" })).getByRole(
        "button",
        { current: true },
      ),
    ).toHaveTextContent(regression.title);
    await expectComparison();

    await user.click(
      screen.getByRole("button", { name: /Can I get a refund/ }),
    );
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
    expect(proxy.get).not.toHaveBeenCalledWith(
      "/lens/evals/runs",
      expect.anything(),
    );
  });

  it("shows why an errored run failed instead of saying it is still scoring", async () => {
    const user = userEvent.setup();
    renderWithLens(<EvalsView />, { searchParams: evalPage });

    const row = await screen.findByRole("row", { name: "flaky-ci@3333333" });
    expect(within(row).getByText("gate errored")).toBeInTheDocument();

    await user.click(row);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "error: dataset revision 9 not found",
    );
    expect(screen.queryByText(/scoring/)).not.toBeInTheDocument();
  });

  it("says so when a PR link names a case outside the run, and clears it on dismiss", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${redRun.id}&eval_case=case-gone`,
      onUrlUpdate,
    });

    const missing = await screen.findByRole("alert");
    expect(missing).toHaveTextContent(
      "Case case-gone was not found in this run.",
    );

    await user.click(within(missing).getByRole("button", { name: "Dismiss" }));
    await expectComparison();
    expect(
      new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has(
        "eval_case",
      ),
    ).toBe(false);
  });

  it("opens a passing unchanged case and its scoped trace while preserving eval navigation", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${mainRun.id}`,
      onUrlUpdate,
    });

    const caseButton = await screen.findByRole("button", {
      name: /Can I get a refund/,
    });
    await user.click(caseButton);
    const traceButton = await screen.findByRole("button", {
      name: "View candidate eval trace",
    });
    expect(screen.getByLabelText("Diagnosis")).not.toHaveTextContent(
      /fixed|regressed/,
    );
    expect(
      screen.queryByRole("button", { name: "View main eval trace" }),
    ).not.toBeInTheDocument();
    await user.click(traceButton);
    expect(
      Object.fromEntries(
        new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString),
      ),
    ).toMatchObject({
      tab: "evals",
      eval: definition.name,
      eval_run: mainRun.id,
      eval_case: regression.case_id,
      trace: "before-trace",
      trace_ref: "before-owner",
    });
    const drawer = await screen.findByRole("complementary", {
      name: "Eval trace details",
    });
    expect(within(drawer).getByText("Eval trace")).toBeInTheDocument();
    expect(within(drawer).getByText(definition.name)).toBeInTheDocument();
    expect(
      await within(drawer).findByRole("button", { name: "Copy trace ID" }),
    ).toHaveAttribute("title", "before-trace");
    await user.click(
      within(drawer).getByRole("button", { name: "Close eval trace (Esc)" }),
    );
    await waitFor(() =>
      expect(
        new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has(
          "trace",
        ),
      ).toBe(false),
    );
    expect(
      new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get(
        "eval_run",
      ),
    ).toBe(mainRun.id);
  });

  it("opens the selected candidate trial, including session references with several traces", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    proxy.get.mockImplementation((path: string, request: GatewayRequest) =>
      path.endsWith("run-red/cases/case-refund")
        ? {
            ...cases["run-red"],
            trials: [
              cases["run-red"]!.trials[0],
              trial(2, [], {
                traces: [
                  {
                    trace_id: "session-first",
                    trace_ref: "session-first-owner",
                  },
                  {
                    trace_id: "session-second",
                    trace_ref: "session-second-owner",
                  },
                ],
              }),
            ],
          }
        : serve(path, request),
    );
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${redRun.id}`,
      onUrlUpdate,
    });

    await user.click(await screen.findByRole("tab", { name: "trial 2" }));
    await user.click(
      screen.getByRole("button", { name: "View candidate eval trace 2" }),
    );
    expect(
      Object.fromEntries(
        new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString),
      ),
    ).toMatchObject({
      trace: "session-second",
      trace_ref: "session-second-owner",
    });
  });

  it("should close the previous case trace when selecting another case", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const secondCase = {
      case_id: "case-shipping",
      title: "Where is my order?",
      critical: false,
      passed: true,
    };
    proxy.get.mockImplementation((path: string, request: GatewayRequest) => {
      if (path === `/lens/evals/runs/${mainRun.id}/cases`)
        return [...serve(path, request), secondCase];
      if (path === `/lens/evals/runs/${mainRun.id}/cases/${secondCase.case_id}`)
        return { ...cases[mainRun.id], ...secondCase };
      return serve(path, request);
    });
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${mainRun.id}&eval_case=${regression.case_id}&trace=before-trace&trace_ref=before-owner&span=old-span&view=thread&span_tab=attributes&steps_q=old&errors=true`,
      onUrlUpdate,
    });
    const drawer = await screen.findByRole("complementary", {
      name: "Eval trace details",
    });

    await user.click(await screen.findByRole("button", { name: /Where is my order/ }));
    await waitFor(() => expect(drawer).toHaveAttribute("data-state", "closing"));
    act(() => fireEvent.animationEnd(drawer));

    await waitFor(() => {
      expect(
        screen.queryByRole("complementary", { name: "Eval trace details" }),
      ).not.toBeInTheDocument();
      expect(
        Object.fromEntries(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString)),
      ).toEqual({
        tab: "evals",
        eval: definition.name,
        eval_run: mainRun.id,
        eval_case: secondCase.case_id,
      });
    });
  });

  it("should clear the open trace before returning to runs and opening another run", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<EvalsView />, {
      searchParams: `${evalPage}&eval_run=${mainRun.id}&eval_case=${regression.case_id}&trace=before-trace&trace_ref=before-owner&fullscreen=true`,
      onUrlUpdate,
    });
    await screen.findByRole("complementary", { name: "Eval trace details" });

    await user.click(screen.getByRole("button", { name: "All runs" }));
    await user.click(await screen.findByRole("row", { name: "drop-policy-check@2222222" }));
    await expectComparison();

    await waitFor(() => {
      expect(
        screen.queryByRole("complementary", { name: "Eval trace details" }),
      ).not.toBeInTheDocument();
      expect(
        Object.fromEntries(new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString)),
      ).toEqual({ tab: "evals", eval: definition.name, eval_run: redRun.id });
    });
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
    fireEvent.change(within(form).getByRole("textbox", { name: "Name" }), {
      target: { value: "refund-order" },
    });
    await user.click(within(form).getByRole("combobox", { name: "Dataset" }));
    await user.click(
      await screen.findByRole("option", { name: "Refund agent regressions@2" }),
    );
    expect(within(form).getByRole("textbox", { name: "Agent" })).toHaveValue(
      "refund-agent",
    );
    await user.click(
      within(form).getByRole("checkbox", { name: "Tool order" }),
    );
    fireEvent.change(
      within(form).getByRole("textbox", { name: "First tool" }),
      {
        target: { value: "check_refund_policy" },
      },
    );
    fireEvent.change(within(form).getByRole("textbox", { name: "Then tool" }), {
      target: { value: "issue_refund" },
    });
    await user.click(within(form).getByRole("button", { name: "Create eval" }));

    await waitFor(() => expect(proxy.put).toHaveBeenCalledOnce());
    expect(proxy.put.mock.lastCall?.[0]).toBe("/lens/evals/refund-order");
    expect(proxy.put.mock.lastCall?.[1].body).toEqual({
      agent: "refund-agent",
      dataset_id: dataset.id,
      scorers: [
        { kind: "task_completed" },
        {
          kind: "called_before",
          first: "check_refund_policy",
          then: "issue_refund",
        },
      ],
      trials: 3,
      baseline: "main",
      gate: {
        regressions: 0,
        critical: 0,
        pass_rate: null,
        cost_per_case: null,
        min: {},
      },
    });
    expect(
      await screen.findByRole("heading", { name: "refund-order" }),
    ).toBeInTheDocument();
    expect(
      new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get(
        "eval",
      ),
    ).toBe("refund-order");
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
    expect(
      within(connect).getByRole("button", { name: "Connect GitHub" }),
    ).toBeEnabled();
    expect(within(connect).queryByRole("tabpanel")).not.toBeInTheDocument();
    await user.click(
      within(connect).getByRole("button", { name: "Connect GitHub" }),
    );
    const dialog = await screen.findByRole("dialog", {
      name: "Connect GitHub",
    });
    expect(
      await within(dialog).findByRole("heading", {
        name: "GitHub connection unavailable",
      }),
    ).toBeVisible();
    expect(
      within(dialog).queryByRole("textbox", { name: "GitHub repository" }),
    ).not.toBeInTheDocument();
    await user.keyboard("{Escape}");

    runs = [redRun];
    await testQueryClient.invalidateQueries();

    await expectComparison();
    expect(
      new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).get(
        "eval_run",
      ),
    ).toBe(redRun.id);
  });

  it("serves an empty run list in sample mode without calling the proxy", async () => {
    renderWithProviders(
      <LensServicesProvider services={createLensDemo()}>
        <RunsTab definition={definition} datasetName={dataset.name} revision={dataset.revision} onOpen={vi.fn()} />
      </LensServicesProvider>,
    );

    expect(
      await screen.findByRole("region", { name: "Connect agent" }),
    ).toBeInTheDocument();
    expect(proxy.get).not.toHaveBeenCalled();
  });
});
