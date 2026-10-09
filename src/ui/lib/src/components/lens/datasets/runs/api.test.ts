import { describe, expect, it, vi } from "vitest";

import type { TracesApi } from "../../traces/api";
import type { Trace } from "../../traces/types";
import { fullTrace, MAX_TRACE_PAGES } from "./api";

const page = (spanId: string, next: string | null): Trace =>
  ({ summary: {}, agents: [], spans: [{ span_id: spanId }], next_cursor: next }) as unknown as Trace;

const tracesApi = (trace: TracesApi["trace"]): TracesApi => ({ trace }) as unknown as TracesApi;

const outcome = { verdict: "fail", trace_id: "trace-pr", trace_ref: "" } as const;

const bounded = (next: (cursor: string | null | undefined) => string): TracesApi["trace"] => {
  const seen: string[] = [];
  return async (_id, _ref, cursor) => {
    seen.push(String(cursor));
    if (seen.length > MAX_TRACE_PAGES + 1) throw new Error("runaway paging");
    return page("s", next(cursor));
  };
};

describe("fullTrace", () => {
  it("follows every cursor and joins the spans in order", async () => {
    const trace = vi.fn<TracesApi["trace"]>(async (_id, _ref, cursor) =>
      cursor === "b" ? page("s2", null) : page("s1", "b"),
    );

    const joined = await fullTrace(tracesApi(trace), outcome);

    expect(joined.spans.map((span) => span.span_id)).toEqual(["s1", "s2"]);
    expect(trace.mock.calls.map(([, , cursor]) => cursor)).toEqual([null, "b"]);
  });

  it("stops when the backend hands back a cursor it already gave", async () => {
    const trace = vi.fn(bounded((cursor) => (cursor === "a" ? "b" : "a")));

    await expect(fullTrace(tracesApi(trace), outcome)).rejects.toThrow("kept paging");
    expect(trace).toHaveBeenCalledTimes(3);
  });

  it("stops after the page cap even when every cursor is new", async () => {
    const trace = vi.fn(bounded(() => crypto.randomUUID()));

    await expect(fullTrace(tracesApi(trace), outcome)).rejects.toThrow("kept paging");
    expect(trace).toHaveBeenCalledTimes(MAX_TRACE_PAGES);
  });
});
