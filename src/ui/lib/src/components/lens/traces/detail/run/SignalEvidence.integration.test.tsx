import { act, fireEvent, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { renderWithProviders, testQueryClient } from "../../../../../../tests/test-utils";
import { Inspector } from "../../../../shared/Inspector";
import research from "../../__fixtures__/research_trace.json";
import { liveTracesApi, TracesApiContext, type TracesApi } from "../../api";
import { AgentTracesTable } from "../../list/AgentTracesTable";
import { traceKey, traceRefOf, useOpenTraceRouting } from "../../routing";
import type { SignalFlag, Span, Trace, TraceSignals } from "../../types";
import { RunView } from "./RunView";

const template = research as Trace;
const root: Span = {
  ...template.spans[0],
  span_id: "root",
  parent_span_id: null,
  name: "Main agent",
};
const branch: Span = {
  ...root,
  span_id: "branch",
  parent_span_id: "root",
  name: "Nested agent",
  type: "agent",
};
const step: Span = {
  ...root,
  span_id: "step",
  parent_span_id: "branch",
  name: "Check result",
  type: "tool",
};
const trace: Trace = {
  ...template,
  spans: [root, branch, step],
  next_cursor: null,
};
const flag: SignalFlag = {
  signal_id: "tool_failure",
  name: "Tool failure",
  score: 0.9,
  evidence: {
    span_id: step.span_id,
    quote: "Output: Cannot open the file\nError: Permission denied",
  },
};

const result = (flags: SignalFlag[]): TraceSignals => ({
  trace_id: trace.summary.trace_id,
  trace_ref: trace.summary.trace_ref ?? "",
  flags,
  status: "classified",
  model: "test",
  classified_at: null,
});

function fixtureApi(flags: SignalFlag[] = [flag]) {
  return {
    ...liveTracesApi("test"),
    live: false,
    handoff: () => ({ text: "Read trace", copied: "Copied" }),
    trace: vi.fn<TracesApi["trace"]>().mockResolvedValue(trace),
    conversation: vi.fn<TracesApi["conversation"]>().mockResolvedValue({ turns: [], next_cursor: null }),
    signals: vi.fn<TracesApi["signals"]>().mockResolvedValue([result(flags)]),
    feedback: vi.fn<TracesApi["feedback"]>().mockResolvedValue({ ...result([]), feedback: [] }),
    span: vi.fn<TracesApi["span"]>().mockImplementation(async (_traceId, spanId) => ({
      span_id: spanId,
      input: "Inspect this file",
      output: spanId === step.span_id ? "Cannot open the file" : "Continue working",
      attributes: {},
    })),
  } satisfies TracesApi;
}

function Harness({ api, flags = [flag] }: { api: TracesApi; flags?: SignalFlag[] }) {
  const routing = useOpenTraceRouting();
  const ref = traceRefOf(trace.summary);
  return (
    <TracesApiContext.Provider value={api}>
      <Inspector.Root
        items={[ref]}
        itemKey={traceKey}
        selected={routing.trace}
        onSelectedChange={routing.openTrace}
        noun="trace"
        storageKey="signal-test"
      >
        <AgentTracesTable
          traces={[trace.summary]}
          findings={new Map()}
          signals={new Map([[traceKey(ref), { status: "ready", signals: result(flags) }]])}
          showSignals
          onOpenSignal={(run, signal) => routing.openSignal(traceRefOf(run), signal)}
          isLoading={false}
          error={null}
          hasMore={false}
          onLoadMore={vi.fn()}
          onSetUpTracing={vi.fn()}
        />
        {routing.trace && (
          <RunView
            traceId={routing.trace.traceId}
            traceRef={routing.trace.traceRef}
            selection={routing.selection}
            accessToken="test"
            onBack={() => routing.openTrace(null)}
            showSignals
          />
        )}
      </Inspector.Root>
    </TracesApiContext.Provider>
  );
}

const tableSignal = () =>
  within(screen.getByTestId("agent-trace-row")).getByRole("button", {
    name: "View Tool failure evidence",
  });
const headerSignal = () =>
  within(screen.getByRole("banner")).getByRole("button", {
    name: "View Tool failure evidence",
  });
const lastUrl = (onUrlUpdate: ReturnType<typeof vi.fn>) =>
  new URLSearchParams(String(onUrlUpdate.mock.lastCall?.[0].queryString ?? ""));

describe("signal evidence navigation", () => {
  beforeEach(() => {
    testQueryClient.clear();
    localStorage.clear();
  });

  it("opens a table signal on its step and highlights the exact retained excerpt without putting text in the URL", async () => {
    const api = fixtureApi();
    const onUrlUpdate = vi.fn();
    renderWithProviders(<Harness api={api} />, { onUrlUpdate });
    await userEvent.click(tableSignal());

    await screen.findByRole("mark");
    const evidence = await screen.findByRole("region", {
      name: "Signal evidence",
    });
    expect(within(evidence).getByRole("heading")).toHaveTextContent(flag.name);
    expect(within(evidence).getByText("Flagged excerpt")).toBeVisible();
    expect(within(evidence).getByRole("mark").textContent).toBe(flag.evidence?.quote);
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", step.span_id);
    expect(await screen.findByRole("region", { name: "Output" })).toHaveTextContent("Cannot open the file");
    await waitFor(() => expect(lastUrl(onUrlUpdate).get("signal")).toBe(flag.signal_id));
    expect(lastUrl(onUrlUpdate).get("trace")).toBe(trace.summary.trace_id);
    expect(lastUrl(onUrlUpdate).get("span")).toBe(step.span_id);
    expect(lastUrl(onUrlUpdate).toString()).not.toContain("Permission");
    expect(api.signals).toHaveBeenCalledOnce();
  });

  it("reveals a collapsed filtered step, reopens closed details, and clears evidence when another step is selected", async () => {
    renderWithProviders(<Harness api={fixtureApi()} />);
    await userEvent.click(screen.getByTestId("agent-trace-row"));
    await screen.findByRole("complementary", { name: "Span details" });
    expect(screen.queryByRole("treeitem", { name: /Check result/ })).not.toBeInTheDocument();
    fireEvent.change(screen.getByRole("textbox", { name: "Search steps" }), {
      target: { value: "no matching step" },
    });
    await userEvent.click(screen.getByRole("button", { name: /^Errors/ }));
    await userEvent.click(screen.getByRole("button", { name: "Close details" }));
    expect(screen.queryByRole("complementary", { name: "Span details" })).not.toBeInTheDocument();

    await userEvent.click(headerSignal());

    expect(await screen.findByRole("complementary", { name: "Span details" })).toBeVisible();
    expect(screen.getByRole("textbox", { name: "Search steps" })).toHaveValue("");
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", step.span_id);
    await userEvent.click(screen.getByRole("button", { name: "Close details" }));
    await userEvent.click(headerSignal());
    expect(await screen.findByRole("region", { name: "Signal evidence" })).toBeVisible();
    await userEvent.click(screen.getByRole("treeitem", { name: /Main agent/ }));
    expect(screen.queryByRole("region", { name: "Signal evidence" })).not.toBeInTheDocument();
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", root.span_id);
  });

  it("restores a signal link through trace pagination and removes stale evidence when the signal disappears", async () => {
    const api = fixtureApi();
    api.trace.mockImplementation(async (_traceId, _traceRef, cursor) =>
      cursor ? { ...trace, spans: [step] } : { ...trace, spans: [root, branch], next_cursor: "more" },
    );
    renderWithProviders(<Harness api={api} />, {
      searchParams: `?trace=${trace.summary.trace_id}&signal=${flag.signal_id}`,
    });
    const evidence = await screen.findByRole("mark");
    expect(evidence.textContent).toBe(flag.evidence?.quote);
    expect(api.trace).toHaveBeenCalledWith(trace.summary.trace_id, undefined, "more");
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", step.span_id);

    api.signals.mockResolvedValue([result([])]);
    await act(() => testQueryClient.invalidateQueries({ queryKey: ["traceSignals"] }));

    await waitFor(() => expect(screen.queryByRole("mark")).not.toBeInTheDocument());
    expect(screen.getByRole("region", { name: "Signal evidence" })).toHaveTextContent(
      "This signal is no longer available for this trace.",
    );
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", root.span_id);
  });

  it.each([
    {
      evidence: undefined,
      message: "No excerpt was recorded for this detection.",
    },
    {
      evidence: { span_id: "missing", quote: "Unavailable source" },
      message: "The flagged step is no longer available in this trace.",
    },
  ])("explains unavailable evidence without showing a misleading quote: $message", async ({ evidence, message }) => {
    const flags = [{ ...flag, evidence }];
    renderWithProviders(<Harness api={fixtureApi(flags)} flags={flags} />);
    await userEvent.click(tableSignal());
    expect(await screen.findByRole("region", { name: "Signal evidence" })).toHaveTextContent(message);
    expect(screen.queryByRole("mark")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Back to runs" }));
    await userEvent.click(screen.getByTestId("agent-trace-row"));
    await screen.findByRole("complementary", { name: "Span details" });
    expect(screen.queryByRole("region", { name: "Signal evidence" })).not.toBeInTheDocument();
  });
});
