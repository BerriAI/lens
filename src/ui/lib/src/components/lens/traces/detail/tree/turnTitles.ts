import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";

import { useTracesApi } from "../../api";
import { sessionTree, turnOpeningCall } from "../../turns";
import type { SpanDetail, Trace } from "../../types";
import { inputMessages } from "../conversation/conversation";
import { newRequestMessages } from "../conversation/sessionCalls";

/** Wrappers Claude Code puts around text it adds to a user message, matched as literal tags. */
const HARNESS_BLOCKS = [
  "system-reminder",
  "local-command-caveat",
  "local-command-stdout",
  "command-name",
  "command-message",
  "command-args",
];

function withoutBlock(text: string, tag: string): string {
  const open = `<${tag}>`;
  const close = `</${tag}>`;
  const start = text.indexOf(open);
  const end = start < 0 ? -1 : text.indexOf(close, start + open.length);
  return end < 0 ? text : withoutBlock(text.slice(0, start) + text.slice(end + close.length), tag);
}

const typedText = (content: string): string => HARNESS_BLOCKS.reduce(withoutBlock, content).replace(/\s+/g, " ").trim();

/** The message a turn answers: the last user text its opening request adds after the last assistant message. */
export function turnTitle(detail: SpanDetail): string {
  return (
    newRequestMessages(inputMessages(detail))
      .flatMap((message) => (message.role === "user" ? [typedText(message.content)] : []))
      .findLast(Boolean) ?? ""
  );
}

/** Titles by turn row id, filled in as each turn's opening request loads. Shares the span cache with the Thread view. */
export function useTurnTitles(trace: Trace, accessToken: string): ReadonlyMap<string, string> {
  const traces = useTracesApi(accessToken);
  const { trace_id: traceId, trace_ref: traceRef } = trace.summary;
  const openings = useMemo(
    () =>
      (sessionTree(trace.spans)?.turns ?? []).flatMap((turn) => {
        const call = turnOpeningCall(turn);
        return call ? [{ id: turn.id, spanId: call.span_id }] : [];
      }),
    [trace.spans],
  );
  const queries = useQueries({
    queries: openings.map(({ spanId }) => ({
      queryKey: ["agentTraceSpan", traceId, traceRef, spanId, accessToken],
      queryFn: ({ signal }: { signal: AbortSignal }): Promise<SpanDetail> =>
        traces.span(traceId, spanId, traceRef, signal),
      staleTime: Infinity,
      retry: false as const,
      retryOnMount: false,
    })),
  });
  return new Map(
    openings.flatMap(({ id }, index) => {
      const detail = queries[index]?.data;
      const title = detail ? turnTitle(detail) : "";
      return title ? [[id, title] as const] : [];
    }),
  );
}
