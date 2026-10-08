import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { testQueryClient } from "@/../tests/test-utils";
import { renderWithLens, stubGateway, type GatewayRequest } from "@/../tests/lens-test-utils";
import type { Span, Trace } from "../../traces/types";
import { DatasetsView } from "../DatasetsView";
import type { Dataset } from "../types";
import type { CaseDiff, EvalRun } from "./types";

vi.mock("@/components/networking", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/components/networking")>()),
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

const regression: CaseDiff = {
  case_id: "case-refund",
  input: "Can I get a refund for order 42?",
  critical: true,
  baseline: { verdict: "pass", trace_id: "trace-main", trace_ref: "" },
  candidate: { verdict: "fail", trace_id: "trace-pr", trace_ref: "" },
};

const mainRun: EvalRun = {
  id: "run-main",
  eval: "refunds",
  agent: "refund-agent",
  dataset_id: dataset.id,
  dataset_revision: 2,
  branch: "main",
  commit_sha: "1111111aaaa",
  pr_url: null,
  status: "finished",
  created_at: "2026-10-01T10:00:00Z",
  finished_at: "2026-10-01T10:05:00Z",
  summary: {
    total: 4,
    passed: 4,
    failed: 0,
    errored: 0,
    pass_rate: 1,
    cost_usd: 0.2,
    baseline_run_id: null,
    baseline_reason: "First run on this revision",
    pass_rate_delta: null,
    regressions: [],
    fixed: [],
  },
  gate: { passed: true, reasons: [] },
};

const redRun: EvalRun = {
  ...mainRun,
  id: "run-pr",
  branch: "drop-policy-check",
  commit_sha: "2222222bbbb",
  pr_url: "https://github.com/example/agent/pull/7",
  created_at: "2026-10-02T10:00:00Z",
  summary: {
    ...mainRun.summary!,
    passed: 3,
    failed: 1,
    pass_rate: 0.75,
    baseline_run_id: mainRun.id,
    baseline_reason: null,
    pass_rate_delta: -0.25,
    regressions: [regression],
  },
  gate: { passed: false, reasons: ["1 critical case regressed against main"] },
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
  } as Trace["summary"],
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
  "": { ...trace("trace-pr", [span("p1", "lookup_order", 10)]), next_cursor: "page-2" },
  "page-2": { ...trace("trace-pr", [span("p2", "issue_refund", 20)]), next_cursor: null },
};

let proxy = stubGateway();

const serve = (path: string, request: GatewayRequest) => {
  if (path === `/lens/datasets/${dataset.id}`) return dataset;
  if (path === "/lens/evals/runs") return [mainRun, redRun];
  if (path === `/lens/evals/runs/${redRun.id}`) return redRun;
  if (path === "/v1/traces/trace-pr") return candidatePages[request.query.cursor ?? ""];
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
  const baseline = await screen.findByRole("list", { name: "Baseline tool steps" });
  const candidate = await screen.findByRole("list", { name: "Candidate tool steps" });
  expect(stepNames(baseline)).toEqual(["lookup_order12ms", "check_refund_policySkipped12ms", "issue_refund12ms"]);
  expect(stepNames(candidate)).toEqual(["lookup_order12ms", "issue_refund12ms"]);
  const comparison = screen.getByRole("region", { name: "Case comparison" });
  expect(comparison).toHaveTextContent(regression.input);
  expect(within(comparison).getByText("Critical")).toBeInTheDocument();
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
    ).toEqual(["3/4", "−25.0 pts", "$0.05", "Gate failed", "Pull request"]);
    expect(within(row).getByRole("link", { name: "Pull request" })).toHaveAttribute("href", redRun.pr_url);
    expect(within(screen.getByRole("region", { name: "Main" })).getByText("Gate passed")).toBeInTheDocument();

    await user.click(within(row).getByRole("button", { name: /drop-policy-check/ }));
    expect(await screen.findByRole("region", { name: "Gate reasons" })).toHaveTextContent(
      "1 critical case regressed against main",
    );
    const summary = screen.getByLabelText("Run summary");
    expect(summary).toHaveTextContent("Passed3/41 failed · 0 errored");
    expect(summary).toHaveTextContent("Regressions10 fixed");
    const regressions = screen.getByRole("region", { name: "Regressions" });
    expect(within(screen.getByRole("region", { name: "Fixed" })).getByText("None")).toBeInTheDocument();

    const regressed = within(regressions).getByRole("button", { name: /Can I get a refund/ });
    expect(regressed).toHaveTextContent("Critical");
    await user.click(regressed);
    await expectComparison();

    const url = onUrlUpdate.mock.lastCall?.[0].queryString as string;
    expect(new URLSearchParams(url).get("eval_case")).toBe(regression.case_id);
    first.unmount();
    testQueryClient.clear();

    renderWithLens(<DatasetsView />, { searchParams: url });
    await expectComparison();
    expect(screen.getByRole("tab", { name: "Runs" })).toHaveAttribute("aria-selected", "true");
  });

  it("lands on the regressed case from a PR comment link that names only the run and case", async () => {
    const user = userEvent.setup();
    renderWithLens(<DatasetsView />, {
      searchParams: `?tab=datasets&dataset=${dataset.id}&eval_run=${redRun.id}&eval_case=${regression.case_id}`,
    });

    await expectComparison();
    expect(proxy.get).not.toHaveBeenCalledWith("/lens/evals/runs", expect.anything());

    await user.click(screen.getByRole("button", { name: "Runs" }));
    expect(await screen.findByRole("region", { name: "Pull requests" })).toBeInTheDocument();
  });
});
