"use client";

import { useQuery } from "@tanstack/react-query";
import { useEffect, useMemo, useRef, useState } from "react";

import { logDetailCall, logListCall } from "../../../../lib/http/requests";
import {
  LIVE_TAIL_INTERVAL_MS,
  type TimeWindow,
} from "../../../shared/timeRange/timeRange";
import type { OtelLogRecord } from "../types";
import { useLogTimelineRouting } from "./routing";

interface LogsTimelineProps {
  accessToken: string;
  enabled: boolean;
  live: boolean;
  range: TimeWindow;
  traceId?: string;
}

const eventName = (event: {
  semantic_event_name: string | null;
  event_name: string;
}): string => event.semantic_event_name || event.event_name || "Unnamed event";

const dateFor = (timestampNs: string): Date =>
  new Date(Number(BigInt(timestampNs) / 1_000_000n));

function CodexEvent({ record }: { record: OtelLogRecord }) {
  const codex = record.codex;
  if (!codex) return null;

  const content = (() => {
    switch (codex.content_state) {
      case "present":
        return codex.content === null ? null : (
          <pre className="whitespace-pre-wrap">{codex.content}</pre>
        );
      case "redacted":
        return <p>Prompt content was redacted by the exporter.</p>;
      case "absent":
        return <p>No message content was captured for this event.</p>;
      case "malformed":
        return <p>The message content had an unsupported type.</p>;
      default:
        return null;
    }
  })();

  return (
    <section className="space-y-2 border-b px-3 py-3 text-sm">
      <h3 className="font-medium">Codex event</h3>
      {codex.message_role && <p>Message role: {codex.message_role}</p>}
      {codex.tool_name && <p>Tool: {codex.tool_name}</p>}
      {codex.call_id && <p>Call ID: {codex.call_id}</p>}
      {typeof codex.success === "boolean" && (
        <p>Tool succeeded: {codex.success ? "yes" : "no"}</p>
      )}
      {content}
      {codex.truncated && <p>Content was truncated by the exporter.</p>}
    </section>
  );
}

function RecordDetail({ record }: { record: OtelLogRecord }) {
  return (
    <section aria-label="Log record details" className="border-t bg-muted/20">
      <CodexEvent record={record} />
      <details className="px-3 py-2">
        <summary className="cursor-pointer text-sm font-medium">
          Raw typed record
        </summary>
        <pre className="mt-2 max-h-96 overflow-auto whitespace-pre-wrap break-words rounded bg-muted p-3 text-xs">
          {JSON.stringify(record, null, 2)}
        </pre>
      </details>
    </section>
  );
}

export function LogsTimeline({
  accessToken,
  enabled,
  live,
  range,
  traceId,
}: LogsTimelineProps) {
  const routing = useLogTimelineRouting();
  const previousWindow = useRef({ startMs: range.startMs, endMs: range.endMs });
  const [traceCursor, setTraceCursor] = useState<string | null>(null);
  const [traceRecordId, setTraceRecordId] = useState<string | null>(null);
  const cursor = traceId ? traceCursor : routing.cursor;
  const recordId = traceId ? traceRecordId : routing.recordId;

  const setCursor = (nextCursor: string | null) => {
    if (traceId) {
      setTraceCursor(nextCursor);
      setTraceRecordId(null);
      return;
    }
    routing.setCursor(nextCursor);
  };

  const setRecordId = (nextRecordId: string | null) => {
    if (traceId) {
      setTraceRecordId(nextRecordId);
      return;
    }
    routing.setRecordId(nextRecordId);
  };

  useEffect(() => {
    if (traceId) return;
    if (
      previousWindow.current.startMs === range.startMs &&
      previousWindow.current.endMs === range.endMs
    )
      return;
    previousWindow.current = { startMs: range.startMs, endMs: range.endMs };
    routing.reset();
  }, [range.startMs, range.endMs, routing.reset, traceId]);

  const page = useQuery({
    queryKey: [
      "otel-log-page",
      accessToken,
      range.startMs,
      range.endMs,
      traceId,
      cursor,
    ],
    queryFn: ({ signal }) =>
      logListCall({
        accessToken,
        startMs: range.startMs,
        endMs: Math.min(range.endMs, Date.now()),
        traceId,
        cursor,
        signal,
      }),
    enabled,
    retry: 1,
    refetchInterval: live && cursor === null ? LIVE_TAIL_INTERVAL_MS : false,
    refetchIntervalInBackground: false,
  });
  const detail = useQuery({
    queryKey: ["otel-log-record", accessToken, traceId, recordId],
    queryFn: ({ signal }) => logDetailCall(accessToken, recordId ?? "", signal),
    enabled: enabled && recordId !== null,
    retry: 1,
  });
  const events = useMemo(() => page.data?.data.toReversed() ?? [], [page.data]);

  return (
    <section
      aria-label={traceId ? "Log events for this trace" : "Log event timeline"}
      className="shrink-0 border-b"
    >
      {page.isLoading ? (
        <p role="status" className="px-3 py-3 text-sm text-muted-foreground">
          Loading log events
        </p>
      ) : page.error ? (
        <div
          role="alert"
          className="flex items-center justify-between gap-4 px-3 py-3 text-sm"
        >
          <p>Could not load log events: {page.error.message}</p>
          <button
            type="button"
            className="underline"
            disabled={page.isFetching}
            onClick={() => void page.refetch()}
          >
            Retry
          </button>
        </div>
      ) : events.length === 0 ? (
        <p className="px-3 py-3 text-sm text-muted-foreground">
          {traceId
            ? "No log events include this trace ID in the selected time range. Unlinked events are available in the global log events timeline."
            : "No log events in this time range."}
        </p>
      ) : (
        <>
          <ol
            aria-label="Log events"
            className="max-h-80 divide-y overflow-auto"
          >
            {events.map((event) => {
              const date = dateFor(event.timestamp_ns);
              return (
                <li key={event.record_id}>
                  <button
                    type="button"
                    aria-pressed={recordId === event.record_id}
                    className="grid w-full grid-cols-[minmax(8rem,auto)_1fr_auto] items-center gap-x-3 px-3 py-2 text-left text-sm hover:bg-muted/50"
                    onClick={() => setRecordId(event.record_id)}
                  >
                    <time
                      dateTime={date.toISOString()}
                      className="text-xs tabular-nums text-muted-foreground"
                    >
                      {date.toLocaleString()}
                    </time>
                    <span className="truncate font-medium">
                      {eventName(event)}
                    </span>
                    <span className="text-xs text-muted-foreground">
                      {event.severity_text ||
                        event.service_name ||
                        event.scope_name}
                    </span>
                  </button>
                </li>
              );
            })}
          </ol>
          <div className="flex items-center justify-between border-t px-3 py-2 text-xs">
            {cursor ? (
              <button
                type="button"
                className="underline"
                onClick={() => setCursor(null)}
              >
                Newest events
              </button>
            ) : (
              <span>{events.length} events loaded</span>
            )}
            {page.data?.next_cursor && (
              <button
                type="button"
                className="ml-auto underline"
                onClick={() => setCursor(page.data?.next_cursor ?? null)}
              >
                Load older events
              </button>
            )}
          </div>
        </>
      )}
      {recordId && detail.isLoading && (
        <p role="status" className="px-3 py-3 text-sm text-muted-foreground">
          Loading log record
        </p>
      )}
      {recordId && detail.error && (
        <p role="alert" className="px-3 py-3 text-sm">
          Could not load log record: {detail.error.message}
        </p>
      )}
      {detail.data && <RecordDetail record={detail.data} />}
    </section>
  );
}
