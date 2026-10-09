import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../../../tests/test-utils";
import {
  renderWithLens,
  stubGateway,
  type GatewayRequest,
} from "../../../../../tests/lens-test-utils";
import { LensServicesProvider } from "../../data/LensServices";
import { createLensDemo } from "../../data/demo/createLensDemo";
import type { Span, Trace } from "../../traces/types";
import { DatasetsView } from "../DatasetsView";
import type { Dataset, DatasetSummary } from "../types";
import { RunsTab } from "./RunsTab";
import type { RunDetails } from "./contract";

vi.mock("../../../../lib/http/requests", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../lib/http/requests")>()),
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

const regression = {
  case_id: "case-refund",
  input: "Can I get a refund for order 42?",
};

const mainRun: RunDetails = {
  run: {
    id: "run-main",
    status: "done",
    eval: "refunds",
    agent: "refund-agent",
    version: "1111111aaaa",
    branch: "main",
    pr: null,
    url: "/ui/?eval_run=run-main",
    expected_trials: 4,
    received_trials: 4,
    failure: "",
    summary: {
      passed: 4,
      total: 4,
      pass_rate: 1,
      cost_per_case: 0.05,
      errors: 0,
      scores: { task_completed: 1 },
      baseline_run_id: null,
      baseline_version: null,
      regressions: [],
      fixed: [],
      gate: { passed: true, reasons: [] },
    },
  },
  dataset_id: dataset.id,
  dataset_revision: 2,
  created_at: "2026-10-01T10:00:00Z",
  completed_at: "2026-10-01T10:05:00Z",
  ci_url: "",
  cases: [
    {
      ...regression,
      verdict: true,
      traces: [{ trace_id: "trace-main", trace_ref: "ref-main" }],
    },
  ],
  baseline: null,
};

const redRun: RunDetails = {
  ...mainRun,
  run: {
    ...mainRun.run,
    id: "run-pr",
    branch: "drop-policy-check",
    version: "2222222bbbb",
    pr: 7,
    summary: {
      ...mainRun.run.summary!,
      passed: 3,
      pass_rate: 0.75,
      baseline_run_id: mainRun.run.id,
      baseline_version: mainRun.run.version,
      regressions: [
        {
          case_id: regression.case_id,
          title: regression.input,
          critical: false,
          baseline_url: mainRun.run.url,
          candidate_url: "/ui/?eval_run=run-pr",
        },
      ],
      gate: { passed: false, reasons: ["pass rate fell below the minimum"] },
    },
  },
  created_at: "2026-10-02T10:00:00Z",
  cases: [
    {
      ...regression,
      verdict: false,
      traces: [{ trace_id: "trace-pr", trace_ref: "ref-pr" }],
    },
  ],
  baseline: { run: mainRun.run, cases: mainRun.cases },
};

const erroredRun: RunDetails = {
  ...redRun,
  run: {
    ...redRun.run,
    id: "run-error",
    branch: "flaky-ci",
    version: "3333333cccc",
    pr: 8,
    status: "failed",
    summary: null,
    failure: "The judge provider rejected the request",
  },
};

const otherDatasetRun: RunDetails = {
  ...redRun,
  run: { ...redRun.run, id: "run-elsewhere" },
  dataset_id: "ds-other",
};

const span = (span_id: string, name: string, start: number): Span => ({
  span_id,
  parent_span_id: "root",
  name,
  type: "tool",
  agent: "refund-agent",
  framework: "",
  start_offset_ms: start,
  duration_ms: 12,
  status: "ok",
  error: null,
  error_truncated: false,
  input_preview: "",
  model: null,
  input_tokens: 0,
  output_tokens: 0,
  litellm_request_id: null,
  spend: null,
  spend_log_request_id: null,
  spend_match: null,
});

const trace = (traceId: string, spans: Span[]): Trace => ({
  summary: {
    trace_id: traceId,
    name: "refund-agent",
    service: "refund-agent",
    input_preview: "",
    start_time: "2026-10-02T10:00:00Z",
    duration_ms: 100,
    status: "ok",
    span_count: spans.length,
    agent_count: 1,
    agent_invocations: 1,
    llm_calls: 0,
    tool_calls: spans.length,
    error_count: 0,
    input_tokens: 0,
    output_tokens: 0,
    models: [],
    priced_calls: 0,
    spend: null,
  },
  agents: [],
  spans,
});

const traces: Record<string, Trace> = {
  "trace-main": trace("trace-main", [
    span("m1", "lookup_order", 10),
    span("m2", "check_refund_policy", 20),
    span("m3", "issue_refund", 30),
  ]),
};

const candidatePages: Record<string, Trace> = {
  "": {
    ...trace("trace-pr", [span("p1", "lookup_order", 10)]),
    next_cursor: "page-2",
  },
  "page-2": {
    ...trace("trace-pr", [span("p2", "issue_refund", 20)]),
    next_cursor: null,
  },
};

let proxy = stubGateway();

const serve = (path: string, request: GatewayRequest) => {
  if (path === "/lens/datasets") return [datasetSummary];
  if (path === `/lens/datasets/${dataset.id}`) return dataset;
  if (
    path === "/lens/evals/runs/details" &&
    request.query.agent === dataset.agent_name
  )
    return [mainRun, redRun, erroredRun];
  const run = [mainRun, redRun, erroredRun, otherDatasetRun].find(
    (item) => path === `/lens/evals/runs/${item.run.id}/details`,
  );
  if (run) return run;
  if (path === "/v1/traces/trace-pr")
    return candidatePages[request.query.cursor ?? ""];
  const traceId = path.match(/^\/v1\/traces\/([^/]+)$/)?.[1];
  if (traceId && traces[traceId]) return traces[traceId];
  throw new Error(`unexpected GET ${path} ${JSON.stringify(request.query)}`);
};

beforeEach(() => {
  testQueryClient.clear();
  proxy = stubGateway();
  proxy.get.mockImplementation(serve);
});

const stepNames = (list: HTMLElement) =>
  within(list)
    .getAllByRole("listitem")
    .map((item) => item.textContent);

async function expectComparison() {
  const baseline = await screen.findByRole("list", {
    name: "Baseline tool steps",
  });
  const candidate = await screen.findByRole("list", {
    name: "Candidate tool steps",
  });
  expect(stepNames(baseline)).toEqual([
    "lookup_order12ms",
    "check_refund_policySkipped12ms",
    "issue_refund12ms",
  ]);
  expect(stepNames(candidate)).toEqual([
    "lookup_order12ms",
    "issue_refund12ms",
  ]);
  const comparison = screen.getByRole("region", { name: "Case comparison" });
  expect(comparison).toHaveTextContent(regression.input);
  expect(within(comparison).queryByText("Critical")).not.toBeInTheDocument();
}

describe("Dataset runs", () => {
  it("opens a red run, compares a regressed case, and comes back to the same view from the URL", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    const first = renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}`,
      onUrlUpdate,
    });

    await user.click(await screen.findByRole("tab", { name: "Runs" }));
    const pulls = await screen.findByRole("region", { name: "Pull requests" });
    const row = within(pulls).getByRole("row", { name: /drop-policy-check/ });
    expect(
      within(row)
        .getAllByRole("cell")
        .slice(1)
        .map((cell) => cell.textContent),
    ).toEqual(["3/4", "−25.0 pts", "$0.05", "Gate failed", "Pull request #7"]);
    expect(within(row).getByText("Pull request #7")).toBeInTheDocument();
    expect(
      within(screen.getByRole("region", { name: "Main" })).getByText(
        "Gate passed",
      ),
    ).toBeInTheDocument();

    await user.click(
      within(row).getByRole("button", { name: /drop-policy-check/ }),
    );
    expect(
      await screen.findByRole("region", { name: "Gate reasons" }),
    ).toHaveTextContent("pass rate fell below the minimum");
    const summary = screen.getByLabelText("Run summary");
    expect(summary).toHaveTextContent("Passed3/41 failed · 0 trial errors");
    expect(summary).toHaveTextContent("Regressions10 fixed");
    const regressions = screen.getByRole("region", { name: "Regressions" });
    expect(
      within(screen.getByRole("region", { name: "Fixed" })).getByText("None"),
    ).toBeInTheDocument();

    const regressed = within(regressions).getByRole("button", {
      name: /Can I get a refund/,
    });
    expect(regressed).not.toHaveTextContent("Critical");
    await user.click(regressed);
    await expectComparison();

    const url = onUrlUpdate.mock.lastCall?.[0].queryString as string;
    expect(new URLSearchParams(url).get("eval_case")).toBe(regression.case_id);
    first.unmount();
    testQueryClient.clear();

    renderWithLens(<DatasetsView />, { searchParams: url });
    await expectComparison();
    expect(screen.getByRole("tab", { name: "Runs" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
  });

  it("lands on the regressed case from a PR comment link that names only the run and case", async () => {
    const user = userEvent.setup();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&eval_run=${redRun.run.id}&eval_case=${regression.case_id}`,
    });

    await expectComparison();
    expect(proxy.get).not.toHaveBeenCalledWith(
      "/lens/evals/runs/details",
      expect.anything(),
    );

    await user.click(screen.getByRole("button", { name: "Runs" }));
    expect(
      await screen.findByRole("region", { name: "Pull requests" }),
    ).toBeInTheDocument();
  });
  it("shows an errored run as errored instead of still scoring", async () => {
    const user = userEvent.setup();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&dataset_tab=runs`,
    });

    const pulls = await screen.findByRole("region", { name: "Pull requests" });
    const row = within(pulls).getByRole("row", { name: /flaky-ci/ });
    expect(within(row).getByText("Gate errored")).toBeInTheDocument();

    await user.click(within(row).getByRole("button", { name: /flaky-ci/ }));
    expect(
      await screen.findByText("The judge provider rejected the request"),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("This run is still being scored."),
    ).not.toBeInTheDocument();
  });

  it("says so when a PR link names a case that did not regress in the run, and clears it on dismiss", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&eval_run=${redRun.run.id}&eval_case=case-gone`,
      onUrlUpdate,
    });

    const missing = await screen.findByRole("alert");
    expect(missing).toHaveTextContent(
      "Case case-gone did not regress or get fixed in this run.",
    );
    expect(
      screen.getByRole("region", { name: "Regressions" }),
    ).toBeInTheDocument();

    await user.click(within(missing).getByRole("button", { name: "Dismiss" }));
    await waitFor(() =>
      expect(screen.queryByRole("alert")).not.toBeInTheDocument(),
    );
    expect(
      new URLSearchParams(onUrlUpdate.mock.lastCall?.[0].queryString).has(
        "eval_case",
      ),
    ).toBe(false);
  });

  it("refuses to show a run from another dataset under this dataset's header", async () => {
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&eval_run=${otherDatasetRun.run.id}&eval_case=${regression.case_id}`,
    });

    expect(
      await screen.findByText("This run belongs to another dataset"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Case comparison" }),
    ).not.toBeInTheDocument();
  });

  it("drops the run and case when you leave the dataset, so reopening it starts on Cases", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&dataset_tab=runs&eval_run=${redRun.run.id}&eval_case=${regression.case_id}`,
      onUrlUpdate,
    });
    await expectComparison();

    await user.click(screen.getByRole("button", { name: "Datasets" }));
    await user.click(
      await screen.findByRole("row", { name: new RegExp(dataset.name) }),
    );

    expect(await screen.findByRole("tab", { name: "Cases" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    const params = new URLSearchParams(
      onUrlUpdate.mock.lastCall?.[0].queryString,
    );
    expect(params.get("dataset")).toBe(dataset.id);
    expect(
      ["dataset_tab", "eval_run", "eval_case"].filter((key) => params.has(key)),
    ).toEqual([]);
  });

  it("opens a persisted candidate trace with its identity and preserves the eval view for Back", async () => {
    const user = userEvent.setup();
    const onUrlUpdate = vi.fn();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&agent=previous-agent&dataset=${dataset.id}&eval_run=${redRun.run.id}&eval_case=${regression.case_id}`,
      onUrlUpdate,
    });
    await user.click(
      await screen.findByRole("button", { name: "Open candidate trace" }),
    );
    await waitFor(() => expect(onUrlUpdate).toHaveBeenCalled());
    const params = new URLSearchParams(
      onUrlUpdate.mock.lastCall?.[0].queryString,
    );
    expect(params.get("tab")).toBe("traces");
    expect(params.get("trace")).toBe("trace-pr");
    expect(params.get("trace_ref")).toBe("ref-pr");
    expect(params.get("agent")).toBe(redRun.run.agent);
    expect(params.get("eval_run")).toBe(redRun.run.id);
    expect(params.get("eval_case")).toBe(regression.case_id);
  });

  it("opens a baseline case even when that case did not change in the baseline run", async () => {
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&eval_run=${mainRun.run.id}&eval_case=${regression.case_id}`,
    });
    const trajectory = await screen.findByRole("region", {
      name: "Candidate trajectory",
    });
    expect(within(trajectory).getByText("Pass")).toBeInTheDocument();
    expect(
      within(trajectory).getByRole("button", { name: "Open candidate trace" }),
    ).toBeInTheDocument();
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("serves an empty run list in sample mode without calling the proxy", async () => {
    renderWithProviders(
      <LensServicesProvider services={createLensDemo()}>
        <RunsTab datasetId={dataset.id} agentName={dataset.agent_name} />
      </LensServicesProvider>,
    );

    expect(await screen.findByText("No eval runs yet")).toBeInTheDocument();
    expect(proxy.get).not.toHaveBeenCalled();
  });
});
