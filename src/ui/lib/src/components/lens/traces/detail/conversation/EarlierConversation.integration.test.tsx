import { screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { readRequest, stubGateway } from "../../../../../../tests/lens-test-utils";
import { renderWithProviders, testQueryClient } from "../../../../../../tests/test-utils";
import research from "../../__fixtures__/research_trace.json";
import { liveTracesApi, TracesApiContext } from "../../api";
import { useLocalRunSelection, useOpenTraceRouting } from "../../routing";
import type { SpanDetail, Trace, TraceConversationPage } from "../../types";
import { RunView } from "../run/RunView";

const currentId = "33333333333333333333333333333333";
const currentRef = "C".repeat(64);
const original = {
  trace_id: "11111111111111111111111111111111",
  trace_ref: "A".repeat(64),
  span_id: "1111111111111111",
  start_time: "2026-10-01T10:00:00Z",
  input: "Find the failing release check",
  output: "The package check failed",
  input_ui: { kind: "text", text: "Find the failing release check" },
  output_ui: { kind: "text", text: "The package check failed" },
} satisfies TraceConversationPage["turns"][number];
const followup = {
  trace_id: "22222222222222222222222222222222",
  trace_ref: "B".repeat(64),
  span_id: "2222222222222222",
  start_time: "2026-10-01T10:10:00Z",
  input: "raw input",
  output: "raw output",
  input_ui: {
    kind: "messages",
    messages: [{ role: "user", content: "Fix that package check", name: null, tool_calls: null }],
  },
  output_ui: {
    kind: "messages",
    messages: [{ role: "assistant", content: "The fix is ready to review", name: null, tool_calls: null }],
  },
} satisfies TraceConversationPage["turns"][number];
const empty: TraceConversationPage = { turns: [], next_cursor: null };
const firstPage: TraceConversationPage = { turns: [original], next_cursor: "next-context-page" };
const lastPage: TraceConversationPage = { turns: [followup], next_cursor: null };
const current: Trace = {
  ...(research as Trace),
  summary: {
    ...research.summary,
    trace_id: currentId,
    trace_ref: currentRef,
    name: "Release verification",
    span_count: 1,
  },
  spans: [{ ...research.spans[0], span_id: "3333333333333333", parent_span_id: null, type: "agent" }],
  next_cursor: null,
};
const currentDetail: SpanDetail = {
  span_id: current.spans[0].span_id,
  input: "Verify the updated release",
  output: "The updated release passes",
  attributes: {},
};
const earlier: Trace = {
  ...current,
  summary: {
    ...current.summary,
    trace_id: original.trace_id,
    trace_ref: original.trace_ref,
    name: "Original release review",
  },
  spans: [{ ...current.spans[0], span_id: original.span_id }],
};

function RoutedRun() {
  const { trace, selection } = useOpenTraceRouting();
  return (
    <RunView
      traceId={trace?.traceId ?? currentId}
      traceRef={trace?.traceRef ?? currentRef}
      selection={selection}
      accessToken="test-tracing-token"
      onBack={vi.fn()}
      embedded
    />
  );
}

function EmbeddedRun() {
  const selection = useLocalRunSelection(null);
  return (
    <RunView
      traceId={currentId}
      traceRef={currentRef}
      selection={selection}
      accessToken="test-tracing-token"
      onBack={vi.fn()}
      embedded
    />
  );
}

function serve() {
  const gateway = stubGateway();
  const history = vi.fn<(cursor?: string) => TraceConversationPage>();
  history.mockReturnValue(empty);
  gateway.get.mockImplementation((path, request) => {
    if (path === `/v1/traces/${currentId}/conversation`) return history(request.query.cursor);
    if (path === `/v1/traces/${original.trace_id}/conversation`) return empty;
    if (path === `/v1/traces/${currentId}`) return current;
    if (path === `/v1/traces/${original.trace_id}`) return earlier;
    if (path === `/v1/traces/${currentId}/spans/${currentDetail.span_id}`) return currentDetail;
    if (path === `/v1/traces/${original.trace_id}/spans/${original.span_id}`)
      return { ...original, attributes: {} } satisfies SpanDetail;
    if (path === "/lens/feedback") return { feedback: [] };
    throw new Error(`Unexpected read: ${path}`);
  });
  return { gateway, history };
}

beforeEach(() => testQueryClient.clear());

describe("Earlier conversation", () => {
  it("opens the original request and reply before the current run from the Steps notice", async () => {
    const user = userEvent.setup();
    const { gateway, history } = serve();
    history.mockReturnValue({ turns: [original], next_cursor: null });
    renderWithProviders(<RoutedRun />);

    expect(await screen.findByRole("tree", { name: "Spans in time order" })).toBeVisible();
    await user.click(await screen.findByRole("button", { name: "View earlier turns" }));
    const thread = await screen.findByRole("region", { name: "Trace thread" });
    expect(screen.getByRole("tab", { name: "Thread", selected: true })).toBeVisible();
    expect(await within(thread).findByText(original.input)).toBeVisible();
    expect(within(thread).getByText(original.output)).toBeVisible();
    expect(await within(thread).findByText(currentDetail.output)).toBeVisible();
    expect(thread).toHaveTextContent(
      /Find the failing release check.*The package check failed.*Current run.*Verify the updated release.*The updated release passes/,
    );
    expect(gateway.get).toHaveBeenCalledWith(`/v1/traces/${currentId}/conversation`, {
      query: { trace_ref: currentRef },
      body: undefined,
      authorization: "Bearer test-tracing-token",
    });
  });

  it("keeps original and current turns while a later context page fails and appends the retry in order", async () => {
    const user = userEvent.setup();
    const { history } = serve();
    history.mockImplementation((cursor) => {
      if (cursor) throw new Error("Conversation unavailable");
      return firstPage;
    });
    renderWithProviders(<RoutedRun />, { searchParams: "?view=thread" });
    const thread = await screen.findByRole("region", { name: "Trace thread" });
    expect(await within(thread).findByText(original.input)).toBeVisible();
    expect(await within(thread).findByText(currentDetail.output)).toBeVisible();
    await user.click(within(thread).getByRole("button", { name: "Load more earlier turns" }));

    expect(await within(thread).findByRole("alert")).toHaveTextContent("This run is still available");
    expect(within(thread).getByText(original.input)).toBeVisible();
    expect(within(thread).getByText(currentDetail.output)).toBeVisible();
    history.mockImplementation((cursor) => (cursor ? lastPage : firstPage));
    await user.click(within(thread).getByRole("button", { name: "Retry conversation" }));

    expect(await within(thread).findByText("The fix is ready to review")).toBeVisible();
    expect(within(thread).getAllByText(original.input)).toHaveLength(1);
    expect(within(thread).getAllByRole("article", { name: "Earlier turn" })).toHaveLength(2);
    expect(thread).toHaveTextContent(
      /Find the failing release check.*The package check failed.*Fix that package check.*The fix is ready to review.*Current run.*Verify the updated release/,
    );
    expect(history.mock.calls.map(([cursor]) => cursor)).toEqual([undefined, "next-context-page", "next-context-page"]);
    expect(within(thread).queryByRole("alert")).not.toBeInTheDocument();
    expect(within(thread).queryByRole("button", { name: "Load more earlier turns" })).not.toBeInTheDocument();
  });

  it("keeps the current trace usable when initial context fails and recovers on request", async () => {
    const user = userEvent.setup();
    const { history } = serve();
    history.mockImplementation(() => {
      throw new Error("Conversation unavailable");
    });
    renderWithProviders(<RoutedRun />);

    expect(await screen.findByRole("alert")).toHaveTextContent("Could not load earlier conversation");
    expect(screen.getByRole("tree", { name: "Spans in time order" })).toBeVisible();
    await user.click(screen.getByRole("tab", { name: "Thread" }));
    const thread = await screen.findByRole("region", { name: "Trace thread" });
    expect(await within(thread).findByText(currentDetail.input)).toBeVisible();
    expect(await within(thread).findByText(currentDetail.output)).toBeVisible();

    history.mockReturnValue({ turns: [original], next_cursor: null });
    await user.click(within(thread).getByRole("button", { name: "Retry conversation" }));
    expect(await within(thread).findByText(original.input)).toBeVisible();
    expect(within(thread).getByText(currentDetail.output)).toBeVisible();
    expect(within(thread).queryByRole("alert")).not.toBeInTheDocument();
  });

  it.each([
    { status: 400, code: "invalid_request" },
    { status: 409, code: "trace_changed" },
  ])(
    "restarts context after a $status cursor failure instead of retrying the stale cursor",
    async ({ status, code }) => {
      const user = userEvent.setup();
      const { history } = serve();
      history.mockReturnValue(firstPage);
      const upstream = globalThis.fetch;
      vi.stubGlobal(
        "fetch",
        vi.fn<typeof fetch>(async (input, init) => {
          const request = await readRequest(input, init);
          if (request.path === `/v1/traces/${currentId}/conversation` && request.query.has("cursor"))
            return Response.json({ detail: { code, message: "Restart this conversation read" } }, { status });
          return upstream(input, init);
        }),
      );
      renderWithProviders(<RoutedRun />, { searchParams: "?view=thread" });
      const thread = await screen.findByRole("region", { name: "Trace thread" });
      await user.click(await within(thread).findByRole("button", { name: "Load more earlier turns" }));
      expect(await within(thread).findByRole("alert")).toHaveTextContent("This run is still available");
      expect(within(thread).getByText(original.input)).toBeVisible();
      expect(await within(thread).findByText(currentDetail.output)).toBeVisible();

      history.mockReturnValue({ turns: [original, followup], next_cursor: null });
      await user.click(within(thread).getByRole("button", { name: "Retry conversation" }));

      expect(await within(thread).findByText("The fix is ready to review")).toBeVisible();
      expect(within(thread).getAllByText(original.input)).toHaveLength(1);
      expect(within(thread).getByText(currentDetail.output)).toBeVisible();
      expect(within(thread).queryByRole("alert")).not.toBeInTheDocument();
      expect(history.mock.calls.map(([cursor]) => cursor)).toEqual([undefined, undefined]);
    },
  );

  it.each([false, true])(
    "links to the exact earlier run in another tab while leaving an embedded run open (demo=%s)",
    async (demo) => {
      const user = userEvent.setup();
      const { history } = serve();
      history.mockReturnValue({ turns: [original], next_cursor: null });
      const onUrlUpdate = vi.fn();
      renderWithProviders(
        <TracesApiContext value={{ ...liveTracesApi("test-tracing-token"), live: !demo }}>
          <EmbeddedRun />
        </TracesApiContext>,
        {
          searchParams: `?tab=evals&eval=release-check&eval_case=case-1&agent=release-agent&custom=keep${demo ? "&demo=true" : ""}`,
          onUrlUpdate,
        },
      );
      await user.click(await screen.findByRole("tab", { name: "Thread" }));
      const context = await screen.findByRole("region", { name: "Earlier conversation" });
      const link = within(context).getByRole("link", { name: "Open earlier run" });
      const href = link.getAttribute("href");
      expect(href).toBeTruthy();
      expect(Object.fromEntries(new URL(href!, "http://localhost/ui/lens").searchParams)).toEqual({
        ...(demo ? { demo: "true" } : {}),
        tab: "traces",
        trace: original.trace_id,
        trace_ref: original.trace_ref,
        view: "thread",
      });
      expect(link).toHaveAttribute("target", "_blank");
      expect(link).toHaveAttribute("rel", "noopener noreferrer");
      await user.click(link);

      expect(await screen.findByText(currentDetail.output)).toBeVisible();
      expect(screen.getByRole("heading", { name: "Release verification" })).toBeVisible();
      expect(screen.getByRole("tab", { name: "Thread", selected: true })).toBeVisible();
      expect(onUrlUpdate).not.toHaveBeenCalled();
    },
  );
});
