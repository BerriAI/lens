import { fireEvent, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { apiClient } from "../../../../lib/http/requests";
import type { OtelLogPage, OtelLogRecord } from "../types";
import {
  renderWithProviders,
  testQueryClient,
} from "../../../../../tests/test-utils";
import { LogsTimeline } from "./LogsTimeline";

const summary = (
  recordId: string,
  timestampNs: string,
  semanticEventName: string,
): OtelLogPage["data"][number] => ({
  record_id: recordId,
  timestamp_ns: timestampNs,
  observed_timestamp_ns: timestampNs,
  severity_number: 0,
  severity_text: "",
  event_name: semanticEventName,
  semantic_event_name: semanticEventName,
  scope_name: "codex_otel.log_only",
  service_name: "codex_cli_rs",
  trace_id: "",
  span_id: "",
  conversation_id: "conversation-1",
});

const record = (
  recordId: string,
  eventName: string,
  codex: NonNullable<OtelLogRecord["codex"]>,
): OtelLogRecord => ({
  record_id: recordId,
  timestamp_ns: "1700000000000000123",
  raw_timestamp_ns: "0",
  observed_timestamp_ns: "1700000000000000456",
  severity_number: 0,
  severity_text: "",
  event_name: eventName,
  semantic_event_name: eventName,
  body: null,
  attributes: {},
  resource_attributes: {},
  resource_schema_url: "",
  scope_name: "codex_otel.log_only",
  scope_version: "",
  scope_attributes: {},
  scope_schema_url: "",
  trace_id: "",
  span_id: "",
  trace_flags: 0,
  dropped_attributes_count: 0,
  service_name: "codex_cli_rs",
  conversation_id: "conversation-1",
  codex,
});

const prompt = record("prompt-record", "codex.user_prompt", {
  event_kind: "user_prompt",
  message_role: "user",
  content_state: "redacted",
  content: null,
  conversation_id: "conversation-1",
  turn_id: null,
  item_id: null,
  call_id: null,
  tool_name: null,
  success: null,
  duration_ms: null,
  truncated: null,
});

const lifecycle = record("sse-record", "codex.sse_event", {
  event_kind: "sse_event",
  message_role: null,
  content_state: null,
  content: null,
  conversation_id: "conversation-1",
  turn_id: null,
  item_id: null,
  call_id: null,
  tool_name: null,
  success: null,
  duration_ms: null,
  truncated: null,
});

const response = record("response-record", "codex.agent_response", {
  event_kind: "agent_response",
  message_role: "assistant",
  content_state: "present",
  content: "A captured response",
  conversation_id: "conversation-1",
  turn_id: "turn-1",
  item_id: "item-1",
  call_id: null,
  tool_name: null,
  success: null,
  duration_ms: null,
  truncated: true,
});

const page = (
  data: OtelLogPage["data"],
  nextCursor: string | null,
): OtelLogPage => ({
  data,
  next_cursor: nextCursor,
});

describe("LogsTimeline", () => {
  afterEach(() => vi.restoreAllMocks());

  beforeEach(() => {
    testQueryClient.clear();
  });

  it("orders events chronologically and opens typed prompt, lifecycle, and response details", async () => {
    const user = userEvent.setup();
    const get = vi
      .spyOn(apiClient, "get")
      .mockImplementation(async (path, options) => {
        if (path === "/lens/logs") {
          return options?.query?.cursor === "older"
            ? page(
                [
                  summary(
                    "response-record",
                    "1700000002000000000",
                    "codex.agent_response",
                  ),
                ],
                null,
              )
            : page(
                [
                  summary(
                    "sse-record",
                    "1700000003000000000",
                    "codex.sse_event",
                  ),
                  summary(
                    "prompt-record",
                    "1700000001000000000",
                    "codex.user_prompt",
                  ),
                ],
                "older",
              );
        }
        if (path === "/lens/logs/prompt-record") return prompt;
        if (path === "/lens/logs/sse-record") return lifecycle;
        if (path === "/lens/logs/response-record") return response;
        throw new Error(`Unexpected request: ${path}`);
      });

    renderWithProviders(
      <LogsTimeline
        accessToken="sk-test"
        enabled
        live={false}
        range={{ startMs: 0, endMs: 10_000 }}
      />,
    );

    const list = await screen.findByRole("list", { name: "Log events" });
    const eventButtons = within(list).getAllByRole("button");
    expect(eventButtons[0]).toHaveTextContent("codex.user_prompt");
    expect(eventButtons[1]).toHaveTextContent("codex.sse_event");
    expect(get).toHaveBeenCalledWith(
      "/lens/logs",
      expect.objectContaining({
        accessToken: "sk-test",
        query: expect.objectContaining({
          start_ms: 0,
          end_ms: 10_000,
          page_size: 50,
        }),
      }),
    );

    await user.click(eventButtons[0]);
    expect(
      await screen.findByText("Prompt content was redacted by the exporter."),
    ).toBeVisible();
    expect(
      screen.getByText(/"timestamp_ns": "1700000000000000123"/),
    ).toBeInTheDocument();
    expect(screen.queryByText("Prompt secret")).not.toBeInTheDocument();

    await user.click(eventButtons[1]);
    expect(await screen.findByText("Raw typed record")).toBeVisible();
    expect(screen.queryByText(/Message role:/)).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Load older events" }));
    const responseButton = await screen.findByRole("button", {
      name: /codex\.agent_response/,
    });
    await user.click(responseButton);
    expect(await screen.findByText("A captured response")).toBeVisible();
    expect(screen.getByText("Message role: assistant")).toBeVisible();
    expect(
      screen.getByText("Content was truncated by the exporter."),
    ).toBeVisible();
    expect(get).toHaveBeenCalledWith(
      "/lens/logs/response-record",
      expect.objectContaining({ accessToken: "sk-test" }),
    );
  });

  it("shows a clear error and retry action when log reads fail", async () => {
    const user = userEvent.setup();
    const get = vi
      .spyOn(apiClient, "get")
      .mockRejectedValue(new Error("storage unavailable"));
    renderWithProviders(
      <LogsTimeline
        accessToken="sk-test"
        enabled
        live={false}
        range={{ startMs: 0, endMs: 10_000 }}
      />,
    );

    expect(
      await screen.findByRole("alert", {}, { timeout: 5_000 }),
    ).toHaveTextContent("Could not load log events: storage unavailable");
    get.mockResolvedValue(page([], null));
    await user.click(screen.getByRole("button", { name: "Retry" }));
    expect(
      await screen.findByText("No log events in this time range."),
    ).toBeVisible();
  });

  it("limits the log timeline to the selected trace", async () => {
    const get = vi
      .spyOn(apiClient, "get")
      .mockResolvedValue(
        page(
          [
            summary(
              "prompt-record",
              "1700000001000000000",
              "codex.user_prompt",
            ),
          ],
          null,
        ),
      );

    renderWithProviders(
      <LogsTimeline
        accessToken="sk-test"
        enabled
        live={false}
        range={{ startMs: 0, endMs: 10_000 }}
        traceId="trace-123"
      />,
    );

    expect(
      await screen.findByRole("button", { name: /codex\.user_prompt/ }),
    ).toBeVisible();
    expect(get).toHaveBeenCalledWith(
      "/lens/logs",
      expect.objectContaining({
        query: expect.objectContaining({
          trace_id: "trace-123",
        }),
      }),
    );
  });

  it("explains when trace-scoped events are unavailable", async () => {
    vi.spyOn(apiClient, "get").mockResolvedValue(page([], null));

    renderWithProviders(
      <LogsTimeline
        accessToken="sk-test"
        enabled
        live={false}
        range={{ startMs: 0, endMs: 10_000 }}
        traceId="trace-123"
      />,
    );

    expect(
      await screen.findByText(
        "No log events include this trace ID in the selected time range. Unlinked events are available in the global log events timeline.",
      ),
    ).toBeVisible();
  });
});
