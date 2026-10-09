"use client";

import { useInfiniteQuery } from "@tanstack/react-query";
import moment from "moment";

import { Button, buttonVariants } from "../../../../ui/button";
import { useTracesApi, useTracesLive } from "../../api";
import { classifyTraceReadFailure, traceReadRetry, traceReadRetryDelay } from "../../list/traceReadFailure";
import type { Trace, TraceConversationPage, UIContent } from "../../types";
import { ConversationMessage } from "./ConversationParts";
import { conversationMessages } from "./conversation";

export function useEarlierConversation(trace: Trace, accessToken: string) {
  const api = useTracesApi(accessToken);
  const { trace_id, trace_ref } = trace.summary;
  return useInfiniteQuery({
    queryKey: ["traceConversation", trace_id, trace_ref, accessToken],
    queryFn: ({ pageParam }) => api.conversation(trace_id, trace_ref, pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    staleTime: 30_000,
    retry: traceReadRetry,
    retryDelay: traceReadRetryDelay,
  });
}

type ConversationHistory = ReturnType<typeof useEarlierConversation>;

export function EarlierConversationNotice({ history, onOpen }: { history: ConversationHistory; onOpen: () => void }) {
  if (history.isError) return <HistoryError history={history} />;
  if (!history.data?.pages.some((page) => page.turns.length)) return null;
  return (
    <div className="flex shrink-0 items-center justify-between gap-3 border-b bg-muted/30 px-4 py-2 text-xs">
      <span>This run has earlier conversation context.</span>
      <Button variant="ghost" size="sm" onClick={onOpen}>
        View earlier turns
      </Button>
    </div>
  );
}

function HistoryError({ history }: { history: ConversationHistory }) {
  const failure = classifyTraceReadFailure(history.error);
  const restart = failure.kind === "invalid" || failure.kind === "changed";
  return (
    <div role="alert" className="flex shrink-0 items-center justify-between gap-3 border-b p-3 text-xs">
      <span>Could not load earlier conversation. This run is still available.</span>
      <Button
        variant="outline"
        size="sm"
        disabled={history.isFetching}
        onClick={() => void (history.isFetchNextPageError && !restart ? history.fetchNextPage() : history.refetch())}
      >
        Retry conversation
      </Button>
    </div>
  );
}

function normalizedContent(content: TraceConversationPage["turns"][number]["input_ui"]): UIContent {
  if (content.kind !== "messages") return content;
  return {
    kind: "messages",
    messages: content.messages.map((message) => ({
      ...message,
      name: message.name ?? undefined,
      tool_calls: message.tool_calls ?? undefined,
    })),
  };
}

export function EarlierConversation({ history }: { history: ConversationHistory }) {
  const live = useTracesLive();
  const turns = history.data?.pages.flatMap((page) => page.turns) ?? [];
  return (
    <>
      {history.isPending && (
        <p role="status" className="text-sm text-muted-foreground">
          Checking earlier conversation…
        </p>
      )}
      {turns.length > 0 && (
        <section aria-label="Earlier conversation" className="min-w-0 space-y-6">
          <div>
            <h2 className="text-sm font-medium">Earlier in this conversation</h2>
            <p className="mt-1 text-xs text-muted-foreground">Recorded turns from earlier runs in this session.</p>
          </div>
          {turns.map((turn) => (
            <article key={`${turn.trace_ref}:${turn.span_id}`} aria-label="Earlier turn" className="min-w-0 space-y-3">
              <div className="flex items-center justify-between gap-3 text-xs text-muted-foreground">
                <time dateTime={turn.start_time}>{moment(turn.start_time).format("MMM D, h:mm A")}</time>
                <a
                  className={buttonVariants({ variant: "ghost", size: "xs" })}
                  href={`?${new URLSearchParams({
                    ...(!live ? { demo: "true" } : {}),
                    tab: "traces",
                    trace: turn.trace_id,
                    trace_ref: turn.trace_ref,
                    view: "thread",
                  })}`}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  Open earlier run
                </a>
              </div>
              {[
                ...conversationMessages(turn.input, normalizedContent(turn.input_ui), "user"),
                ...conversationMessages(turn.output, normalizedContent(turn.output_ui), "assistant"),
              ].map((message, index) => (
                <ConversationMessage key={index} message={message} />
              ))}
            </article>
          ))}
        </section>
      )}
      {history.isError && <HistoryError history={history} />}
      {history.hasNextPage && !history.isError && (
        <Button variant="outline" size="sm" disabled={history.isFetching} onClick={() => void history.fetchNextPage()}>
          {history.isFetchingNextPage ? "Loading conversation…" : "Load more earlier turns"}
        </Button>
      )}
      {turns.length > 0 && <h2 className="border-t pt-4 text-sm font-medium">Current run</h2>}
    </>
  );
}
