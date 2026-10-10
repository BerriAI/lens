import { act, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithLens, stubGateway } from "../../../../../tests/lens-test-utils";
import { testQueryClient } from "../../../../../tests/test-utils";
import { createLensDemoData } from "../../data/demo/fixtures";
import { RunDetail } from "./RunDetail";
import { evalRun, runCase, summary, trial } from "./testRuns";
import type { EvalRun, RunCase } from "./types";

vi.mock("../../../../lib/http/requests", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../../../../lib/http/requests")>()),
  proxyBaseUrl: "",
  getProxyBaseUrl: () => "",
}));

const completed = evalRun("run-one", {
  summary: summary({ passed: 1, total: 2, regressions: [], fixed: [], gate: { passed: true, reasons: [] } }),
});
const passed = { ...runCase(true, [trial(1, [])]), case_id: "passed-case", title: "Preserves existing files" };
const failed = { ...runCase(false, [trial(1, [])]), case_id: "failed-case", title: "Returns the expected answer" };
let gateway = stubGateway();

beforeEach(() => {
  testQueryClient.clear();
  gateway = stubGateway();
});

function serveRun(run: EvalRun, cases: readonly RunCase[]) {
  gateway.get.mockImplementation((path) => {
    if (path === `/lens/evals/runs/${run.id}`) return run;
    if (path === `/lens/evals/runs/${run.id}/cases`) return cases;
    const item = cases.find((entry) => path === `/lens/evals/runs/${run.id}/cases/${entry.case_id}`);
    if (item) return item;
    throw new Error(`Unexpected path ${path}`);
  });
}

describe("Eval case results", () => {
  it("should show failed run checks even when every case passes", async () => {
    const run = evalRun("policy-failed", {
      summary: summary({
        passed: 1,
        total: 1,
        gate: { passed: false, reasons: ["Cost per case exceeded the configured limit"] },
      }),
    });
    serveRun(run, [passed]);
    renderWithLens(<RunDetail runId={run.id} caseId={null} onBack={vi.fn()} onOpenCase={vi.fn()} />);

    const header = await screen.findByRole("banner", { name: "Run" });
    expect(within(header).getByText("Checks failed")).toBeVisible();
    expect(screen.getByLabelText("Case totals")).toHaveTextContent("1 passed0 failed1 cases");
    expect(within(header).getByRole("alert")).toHaveTextContent("Cost per case exceeded the configured limit");
    const list = await screen.findByRole("navigation", { name: "Test cases" });
    expect(within(list).getByRole("button", { name: /Preserves existing files/ })).toHaveTextContent("Passed");
  });

  it("should show every case and real verdicts even when the regression policy passes", async () => {
    const onOpenCase = vi.fn();
    const user = userEvent.setup();
    serveRun(completed, [passed, failed]);
    renderWithLens(<RunDetail runId={completed.id} caseId={null} onBack={vi.fn()} onOpenCase={onOpenCase} />);

    const list = await screen.findByRole("navigation", { name: "Test cases" });
    expect(within(list).getByRole("button", { name: /Preserves existing files/ })).toHaveTextContent("Passed");
    expect(within(list).getByRole("button", { name: /Returns the expected answer/ })).toHaveTextContent("Failed");
    expect(within(screen.getByRole("banner", { name: "Run" })).getByText("Failed")).toBeInTheDocument();
    expect(screen.getByLabelText("Case totals")).toHaveTextContent("1 passed1 failed2 cases");
    await user.click(within(list).getByRole("button", { name: /Preserves existing files/ }));
    expect(onOpenCase).toHaveBeenCalledWith(passed.case_id);
  });

  it("should open the linked trace with its exact source reference", async () => {
    const data = createLensDemoData();
    const record = data.runs[0];
    const traceRef = "evaluation-source-reference";
    const linked = {
      ...passed,
      trials: [trial(1, [], { traces: [{ trace_id: record.trace.summary.trace_id, trace_ref: traceRef }] })],
    };
    gateway.get.mockImplementation((path) => {
      if (path === `/lens/evals/runs/${completed.id}`) return completed;
      if (path === `/lens/evals/runs/${completed.id}/cases`) return [linked];
      if (path === `/lens/evals/runs/${completed.id}/cases/${linked.case_id}`) return linked;
      if (path === `/v1/traces/${record.trace.summary.trace_id}/conversation`) return { turns: [], next_cursor: null };
      if (path === `/v1/traces/${record.trace.summary.trace_id}`)
        return { ...record.trace, summary: { ...record.trace.summary, trace_ref: traceRef } };
      if (path === "/lens/feedback") return { feedback: [] };
      const detail = record.details.find((span) => path.endsWith(`/spans/${span.span_id}`));
      if (detail) return detail;
      throw new Error(`Unexpected path ${path}`);
    });
    renderWithLens(<RunDetail runId={completed.id} caseId={linked.case_id} onBack={vi.fn()} onOpenCase={vi.fn()} />);

    expect(await screen.findByRole("button", { name: "Copy trace ID" })).toHaveAttribute(
      "title",
      record.trace.summary.trace_id,
    );
    expect(gateway.get).toHaveBeenCalledWith(
      `/v1/traces/${record.trace.summary.trace_id}`,
      expect.objectContaining({
        query: expect.objectContaining({ trace_ref: traceRef }),
        authorization: "Bearer test",
      }),
    );
    expect(screen.queryByText("Trace unavailable")).not.toBeInTheDocument();
  });

  it("should refresh a selected pending case when the run finishes", async () => {
    let finished = false;
    gateway.get.mockImplementation((path) => {
      const run = finished ? completed : { ...completed, status: "scoring", summary: null };
      const item = finished ? failed : { ...failed, passed: null, trials: [] };
      if (path === `/lens/evals/runs/${completed.id}`) return run;
      if (path === `/lens/evals/runs/${completed.id}/cases`) return [item];
      if (path === `/lens/evals/runs/${completed.id}/cases/${failed.case_id}`) return item;
      throw new Error(`Unexpected path ${path}`);
    });
    renderWithLens(<RunDetail runId={completed.id} caseId={failed.case_id} onBack={vi.fn()} onOpenCase={vi.fn()} />);

    const result = await screen.findByRole("region", { name: "Case result" });
    expect(await within(result).findByText("Waiting for this case to finish")).toBeInTheDocument();
    expect(within(result).getByText("Pending")).toBeInTheDocument();
    finished = true;
    await act(() => testQueryClient.invalidateQueries({ predicate: (query) => query.queryKey.includes("detail") }));
    expect(await screen.findByText("Trace unavailable")).toBeInTheDocument();
    const completedResult = screen.getByRole("region", { name: "Case result" });
    expect(within(completedResult).getByText("Failed")).toBeInTheDocument();
    expect(within(completedResult).queryByText("Pending")).not.toBeInTheDocument();
  });

  it("should distinguish unscored cases and execution errors from passing cases", async () => {
    const unscored = {
      ...failed,
      passed: null,
      trials: [trial(1, [], { error: "Agent process exited before returning output" })],
    };
    serveRun(completed, [unscored]);
    renderWithLens(<RunDetail runId={completed.id} caseId={unscored.case_id} onBack={vi.fn()} onOpenCase={vi.fn()} />);

    expect(await screen.findByRole("button", { name: /Returns the expected answer/ })).toHaveTextContent("Not scored");
    const result = await screen.findByRole("region", { name: "Case result" });
    expect(await within(result).findByText("Error")).toBeInTheDocument();
    expect(within(result).getByRole("alert")).toHaveTextContent("Agent process exited before returning output");
  });
});
