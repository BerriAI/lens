import assert from "node:assert/strict";
import { test } from "node:test";
import { Runner } from "@openai/agents";
import {
  ScriptedModel,
  assistantMessage,
  functionCall,
} from "@openai/agents/testing";
import { responder } from "./agent.js";
import type { AgentConfig as Config } from "./config.js";
import { LensClient, type TraceWindow } from "./lens.js";
import type { FindingContext as FindingThread } from "./models.js";
import { replyTools } from "./tools.js";

const config = { agent: "selected", model: "test-model" } as Config;

test("finding citations count each trace once while retaining different cited spans and their original labels", async () => {
  const scoped = { ...config, publicUrl: "https://lens.example.com" };
  const trace = (id: string, span: string) => ({
    trace_id: id,
    trace_ref: `ref-${id}`,
    span_id: span,
    agent_names: ["selected"],
    service: "selected",
    start_time: "2026-01-01",
    duration_ms: 100,
    status: id === "failed" ? "error" : "ok",
    llm_calls: 0,
    tool_calls: 1,
    error_count: id === "failed" ? 1 : 0,
    input_tokens: 0,
    output_tokens: 0,
    url: `https://lens.example.com/?trace=${id}&span=${span}`,
  });
  const requests: string[] = [];
  const result = await replyTools(
    scoped,
    {
      read: async () => assert.fail("A finding must not read unrelated traces"),
      link: (query) =>
        `https://lens.example.com/?${new URLSearchParams(query)}`,
      get: async (path) => {
        requests.push(path);
        const url = new URL(path, scoped.publicUrl);
        const id = url.pathname.split("/")[3]!;
        const span = url.pathname.split("/")[5];
        if (span)
          return {
            span_id: span,
            input: JSON.stringify([
              { role: "user", content: "Inspect repository" },
            ]),
            output: "Recorded result",
            attributes: { "gen_ai.operation.name": "invoke_agent" },
          };
        return {
          summary: { trace_id: id, agent_names: ["selected"] },
          spans: [
            {
              span_id: "root",
              parent_span_id: null,
              type: "agent",
              name: "selected",
              status: id === "failed" ? "error" : "ok",
              duration_ms: 100,
              error: null,
            },
            {
              span_id: "tool",
              parent_span_id: "root",
              type: "tool",
              name: "repository_lookup",
              status: id === "failed" ? "error" : "ok",
              duration_ms: 90,
              error: id === "failed" ? "HTTP 503" : null,
            },
          ],
        };
      },
    },
    AbortSignal.timeout(10_000),
    {
      issue: "fixture",
      title: "Repository lookup error",
      traces: [
        trace("failed", "root"),
        trace("failed", "tool"),
        trace("failed", "tool"),
        trace("healthy", "root"),
      ],
    },
  );
  const context = result.traceContext!;
  assert.equal(context.report.population.terminal_traces, 2);
  assert.equal(context.report.population.root_error_traces, 1);
  assert.deepEqual(
    context.report.metrics.find((metric) => metric.id === "root_errors")
      ?.affected_trace_ids,
    ["failed"],
  );
  assert.equal(
    context.report.metrics.find((metric) => metric.id === "root_errors")?.total,
    2,
  );
  assert.equal(
    context.report.metrics.find(
      (metric) => metric.title === "repository_lookup calls fail",
    )?.count,
    1,
  );
  assert.equal(
    context.report.metrics.find(
      (metric) => metric.title === "repository_lookup calls fail",
    )?.total,
    2,
  );
  assert.deepEqual(
    context.numbered_sources.map((source) => source.label),
    ["Trace 1", "Trace 2", "Trace 4"],
  );
  assert.equal(context.cited_spans.length, 3);
  assert.equal(context.traces.length, 2);
  assert.equal(
    requests.filter((path) => path.startsWith("/v1/traces/failed?")).length,
    1,
  );
  assert.equal(
    requests.filter((path) => path.startsWith("/v1/traces/failed/spans/root?"))
      .length,
    1,
  );
  await assert.rejects(
    replyTools(
      scoped,
      { read: async () => assert.fail("Unexpected read") },
      AbortSignal.timeout(10_000),
      {
        issue: "fixture",
        title: "Conflicting revisions",
        traces: [
          trace("failed", "root"),
          { ...trace("failed", "root"), trace_ref: "another-revision" },
        ],
      },
    ),
    /conflicting trace revisions/,
  );
});

test("official SDK executes the read tool, carries its evidence to the model, and bounds repeated tool reads", async () => {
  const model = new ScriptedModel([
    [functionCall("findings", {}, { callId: "first" })],
    [functionCall("findings", {}, { callId: "repeat" })],
    [
      assistantMessage(
        JSON.stringify({
          title: "Candidate",
          summary: "One candidate; no measured gain is established",
          opportunities: [],
          sources: [],
        }),
      ),
    ],
  ]);
  let reads = 0;
  const answer = await responder(
    config,
    {
      read: async () => {
        reads++;
        return JSON.stringify({ candidate: "fixture evidence" });
      },
    },
    new Runner({
      modelProvider: { getModel: async () => model },
      tracingDisabled: true,
    }),
  )(
    [{ role: "user", content: "What should improve?" }],
    AbortSignal.timeout(10_000),
  );
  assert.equal(reads, 1);
  assert.equal(model.calls.length, 3);
  assert(
    JSON.stringify(model.calls[1]?.request.input).includes("fixture evidence"),
  );
  assert(answer.answer.summary.includes("no measured gain"));
  assert.equal(model.firstCall?.request.modelSettings.store, false);
  model.assertComplete();
});

test("SDK turn limit stops a model that continues asking for evidence", async () => {
  const model = new ScriptedModel(
    Array.from({ length: 12 }, (_, i) => [
      functionCall("recent_traces", {}, { callId: String(i) }),
    ]),
  );
  const answer = responder(
    config,
    { read: async () => "bounded evidence" },
    new Runner({
      modelProvider: { getModel: async () => model },
      tracingDisabled: true,
    }),
  );
  await assert.rejects(
    answer(
      [{ role: "user", content: "continue forever" }],
      AbortSignal.timeout(10_000),
    ),
    /Max turns/i,
  );
  assert.equal(model.calls.length, 8);
});

test("SDK passes the requested lookback into the evidence reader and preserves its exact window", async () => {
  const window = { lookback_hours: 12, start: null, end: null };
  const label =
    "Traces started from 2026-01-02T00:00:00.000Z (inclusive) to 2026-01-02T12:00:00.000Z (exclusive)";
  const model = new ScriptedModel([
    [functionCall("recent_traces", window, { callId: "window" })],
    [
      assistantMessage(
        JSON.stringify({
          title: "Last 12 hours",
          summary: "No completed tasks were observed in the requested window",
          opportunities: [],
          sources: [],
        }),
      ),
    ],
  ]);
  const windows: (TraceWindow | undefined)[] = [];
  await responder(
    config,
    {
      read: async (name, _signal, selected) => {
        assert.equal(name, "recent_traces");
        windows.push(selected);
        return JSON.stringify({ window: label, records: [], metrics: [] });
      },
    },
    new Runner({
      modelProvider: { getModel: async () => model },
      tracingDisabled: true,
    }),
  )(
    [{ role: "user", content: "Analyze traces for the last 12 hours" }],
    AbortSignal.timeout(10_000),
  );
  assert.deepEqual(windows, [window]);
  assert(JSON.stringify(model.calls[1]?.request.input).includes(label));
  model.assertComplete();
});

test("a finding follow-up reads its exact live trace and observed span, rejecting attempts to read a different trace", async () => {
  const scoped = {
    ...config,
    publicUrl: "https://lens.example.com",
    apiUrl: "http://127.0.0.1",
  };
  const url =
    "https://lens.example.com/?agent=selected&tab=traces&trace=chosen&trace_ref=saved-ref";
  const finding: FindingThread = {
    issue: "12",
    title: "Reliability: Lookup fails",
    traces: [
      {
        trace_id: "chosen",
        trace_ref: "saved-ref",
        agent_names: ["selected"],
        service: "selected",
        start_time: "2026-01-01",
        duration_ms: 180000,
        status: "error",
        llm_calls: 1,
        tool_calls: 1,
        error_count: 1,
        input_tokens: 1,
        output_tokens: 1,
        url,
      },
    ],
  };
  const requests: string[] = [];
  const client = new LensClient(scoped, async (target) => {
    const request = new URL(String(target));
    requests.push(request.pathname);
    assert.equal(request.searchParams.get("trace_ref"), "saved-ref");
    assert(request.pathname.startsWith("/v1/traces/chosen"));
    if (request.pathname.includes("/spans/"))
      return new Response(
        JSON.stringify({
          span_id: request.pathname.endsWith("/tool") ? "tool" : "root",
          input: JSON.stringify([
            { role: "user", content: "Please inspect this repository" },
          ]),
          output: request.pathname.endsWith("/tool")
            ? "Tool failed (HTTP 503). The action was not confirmed. sk-credential"
            : "Repository access unavailable",
          attributes: { "gen_ai.operation.name": "invoke_agent" },
        }),
      );
    return new Response(
      JSON.stringify({
        summary: { trace_id: "chosen", agent_names: ["selected"] },
        spans: [
          {
            span_id: "root",
            parent_span_id: null,
            type: "agent",
            name: "selected",
            status: "error",
            duration_ms: 181000,
            error: null,
          },
          {
            span_id: "tool",
            parent_span_id: "root",
            type: "tool",
            name: "github_repositories",
            status: "error",
            duration_ms: 180000,
            error: "HTTP 503",
          },
        ],
      }),
    );
  });
  const model = new ScriptedModel([
    [
      functionCall(
        "trace_span",
        { trace_id: "unrelated", span_id: "tool" },
        { callId: "outside" },
      ),
    ],
    [
      functionCall(
        "trace_span",
        { trace_id: "chosen", span_id: "tool" },
        { callId: "inside" },
      ),
    ],
    [
      assistantMessage(
        JSON.stringify({
          title: "Reliability",
          summary: "Repository access returned HTTP 503 after 180 seconds",
          opportunities: [],
          sources: [{ label: "Trace 1", url }],
        }),
      ),
    ],
  ]);
  const answer = await responder(
    scoped,
    client,
    new Runner({
      modelProvider: { getModel: async () => model },
      tracingDisabled: true,
    }),
  )(
    [{ role: "user", content: "Why did this fail?" }],
    AbortSignal.timeout(10_000),
    finding,
  );
  assert.deepEqual(requests, [
    "/v1/traces/chosen",
    "/v1/traces/chosen/spans/root",
    "/v1/traces/chosen/spans/tool",
  ]);
  assert(
    JSON.stringify(model.firstCall?.request.input).includes(
      "Please inspect this repository",
    ),
  );
  const lastInput = JSON.stringify(model.calls[2]?.request.input);
  assert(lastInput.includes("The action was not confirmed"));
  assert(!lastInput.includes("sk-credential"));
  assert.equal(answer.answer.sources[0]?.url, url);
  assert(answer.evidenceUrls.includes(url));
  model.assertComplete();
});
