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

function Harness({ api, flags = [flag], runs = [trace] }: { api: TracesApi; flags?: SignalFlag[]; runs?: Trace[] }) {
  const routing = useOpenTraceRouting();
  const refs = runs.map((run) => traceRefOf(run.summary));
  return (
    <TracesApiContext.Provider value={api}>
      <Inspector.Root
        items={refs}
        itemKey={traceKey}
        selected={routing.trace}
        onSelectedChange={routing.openTrace}
        noun="trace"
        storageKey="signal-test"
      >
        <AgentTracesTable
          traces={runs.map((run) => run.summary)}
          findings={new Map()}
          signals={new Map(refs.map((ref) => [traceKey(ref), { status: "ready", signals: result(flags) }]))}
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

  it("opens an ordinary flagged row on the first signal with evidence and highlights its exact excerpt", async () => {
    const flags = [{ ...flag, signal_id: "legacy", name: "Legacy detection", evidence: undefined }, flag];
    const api = fixtureApi(flags);
    const onUrlUpdate = vi.fn();
    renderWithProviders(<Harness api={api} flags={flags} />, { onUrlUpdate });

    await userEvent.click(screen.getByTestId("agent-trace-row"));

    const evidence = await screen.findByRole("region", { name: "Signal evidence" });
    expect(within(evidence).getByRole("heading")).toHaveTextContent(flag.name);
    expect(within(evidence).getByRole("mark").textContent).toBe(flag.evidence?.quote);
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", step.span_id);
    expect(await screen.findByRole("region", { name: "Output" })).toHaveTextContent("Cannot open the file");
    await waitFor(() => expect(lastUrl(onUrlUpdate).get("signal")).toBe(flag.signal_id));
    expect(lastUrl(onUrlUpdate).get("span")).toBe(step.span_id);
    expect(lastUrl(onUrlUpdate).toString()).not.toContain("Permission");
  });

  it("preserves a direct step link when the trace also has signal evidence", async () => {
    renderWithProviders(<Harness api={fixtureApi()} />, {
      searchParams: `?trace=${trace.summary.trace_id}&span=${root.span_id}`,
    });

    await within(await screen.findByRole("banner")).findByRole("button", { name: "View Tool failure evidence" });

    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", root.span_id);
    expect(screen.queryByRole("region", { name: "Signal evidence" })).not.toBeInTheDocument();
  });

  it("keeps a manually selected step when signal evidence arrives later", async () => {
    const pending = Promise.withResolvers<TraceSignals[]>();
    const api = fixtureApi();
    api.signals.mockReturnValue(pending.promise);
    renderWithProviders(<Harness api={api} />);
    await userEvent.click(screen.getByTestId("agent-trace-row"));
    await screen.findByRole("complementary", { name: "Span details" });
    await userEvent.click(screen.getByRole("treeitem", { name: /Nested agent/ }));

    await act(() => pending.resolve([result([flag])]));
    await within(screen.getByRole("banner")).findByRole("button", { name: "View Tool failure evidence" });

    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", branch.span_id);
    expect(screen.queryByRole("region", { name: "Signal evidence" })).not.toBeInTheDocument();
  });

  it.each(["pointer", "keyboard"])("keeps details closed by %s when signal evidence arrives later", async (input) => {
    const pending = Promise.withResolvers<TraceSignals[]>();
    const api = fixtureApi();
    api.signals.mockReturnValue(pending.promise);
    renderWithProviders(<Harness api={api} />);
    await userEvent.click(screen.getByTestId("agent-trace-row"));
    await screen.findByRole("complementary", { name: "Span details" });
    if (input === "pointer") await userEvent.click(screen.getByRole("button", { name: "Close details" }));
    else fireEvent.keyDown(document.body, { key: "Escape" });
    expect(screen.queryByRole("complementary", { name: "Span details" })).not.toBeInTheDocument();

    await act(() => pending.resolve([result([flag])]));
    await within(screen.getByRole("banner")).findByRole("button", { name: "View Tool failure evidence" });

    expect(screen.queryByRole("complementary", { name: "Span details" })).not.toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Signal evidence" })).not.toBeInTheDocument();
  });

  it("keeps dismissed evidence closed after refetch and initializes it again when the run is reopened", async () => {
    const api = fixtureApi();
    renderWithProviders(<Harness api={api} />);
    await userEvent.click(screen.getByTestId("agent-trace-row"));
    const evidence = await screen.findByRole("region", { name: "Signal evidence" });
    await userEvent.click(within(evidence).getByRole("button", { name: "Dismiss signal evidence" }));

    await act(() => testQueryClient.invalidateQueries({ queryKey: ["traceSignals"] }));

    expect(screen.queryByRole("region", { name: "Signal evidence" })).not.toBeInTheDocument();
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", step.span_id);
    await userEvent.click(screen.getByRole("button", { name: "Back to runs" }));
    await userEvent.click(screen.getByTestId("agent-trace-row"));
    expect((await screen.findByRole("mark")).textContent).toBe(flag.evidence?.quote);
  });

  it("initializes the next run after evidence was dismissed in the previous run", async () => {
    const next = { ...trace, summary: { ...trace.summary, trace_id: "next-run", trace_ref: "next-ref" } };
    const api = fixtureApi();
    api.trace.mockImplementation(async (traceId) => (traceId === next.summary.trace_id ? next : trace));
    api.signals.mockImplementation(async (traces) => traces.map((identity) => ({ ...result([flag]), ...identity })));
    const onUrlUpdate = vi.fn();
    renderWithProviders(<Harness api={api} runs={[trace, next]} />, { onUrlUpdate });
    await userEvent.click(screen.getAllByTestId("agent-trace-row")[0]);
    const evidence = await screen.findByRole("region", { name: "Signal evidence" });
    await userEvent.click(within(evidence).getByRole("button", { name: "Dismiss signal evidence" }));
    expect(screen.queryByRole("mark")).not.toBeInTheDocument();

    await userEvent.click(screen.getAllByTestId("agent-trace-row")[1]);

    expect((await screen.findByRole("mark")).textContent).toBe(flag.evidence?.quote);
    await waitFor(() => expect(lastUrl(onUrlUpdate).get("trace")).toBe(next.summary.trace_id));
    expect(lastUrl(onUrlUpdate).get("signal")).toBe(flag.signal_id);
    expect(screen.getByRole("treeitem", { selected: true })).toHaveAttribute("data-row-id", step.span_id);
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
    renderWithProviders(<Harness api={fixtureApi()} />, {
      searchParams: `?trace=${trace.summary.trace_id}&span=${root.span_id}`,
    });
    await screen.findByRole("complementary", { name: "Span details" });
    await userEvent.click(
      within(screen.getByRole("treeitem", { name: /Nested agent/ })).getByRole("button", { name: "Collapse" }),
    );
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

    expect(screen.queryByRole("mark")).not.toBeInTheDocument();
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
    expect(await screen.findByRole("region", { name: "Signal evidence" })).toHaveTextContent(message);
    expect(screen.queryByRole("mark")).not.toBeInTheDocument();
  });
});
