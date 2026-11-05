import { expectTypeOf, test } from "vitest";
import type {
  SpanErrorQuery,
  SpanQuery,
  TraceDetailQuery,
  TraceListQuery,
  TraceMessage,
  TraceQueryBody,
  UIMessage,
  LogListRequest,
} from "./types";

test("trace messages accept API names while adapting tool calls", () => {
  expectTypeOf<TraceMessage["name"]>().toEqualTypeOf<UIMessage["name"]>();
  expectTypeOf<UIMessage["content"]>().toEqualTypeOf<TraceMessage["content"]>();
});

test("trace request aliases match the generated OpenAPI shapes", () => {
  expectTypeOf<TraceListQuery>().toEqualTypeOf<{
    start_ms?: number | null;
    end_ms?: number | null;
    cursor?: string | null;
    agent?: string | null;
  }>();
  expectTypeOf<TraceDetailQuery>().toEqualTypeOf<{
    trace_ref?: string;
    cursor?: string | null;
    page_size?: number | null;
  }>();
  expectTypeOf<SpanQuery>().toEqualTypeOf<{ trace_ref?: string }>();
  expectTypeOf<SpanErrorQuery>().toEqualTypeOf<{
    trace_ref?: string;
    cursor?: string | null;
  }>();
  expectTypeOf<TraceQueryBody>().toEqualTypeOf<{ sql: string }>();
});

test("log list query fields match the Rust contract", () => {
  expectTypeOf<LogListRequest>().toEqualTypeOf<{
    start_ms?: number | null;
    end_ms?: number | null;
    service?: string | null;
    event?: string | null;
    severity?: string | null;
    trace_id?: string | null;
    conversation_id?: string | null;
    cursor?: string | null;
    page_size?: number | null;
    [k: string]: unknown;
  }>();
});
