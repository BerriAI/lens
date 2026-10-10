import assert from "node:assert/strict";
import { test } from "node:test";
import { LensClient } from "./lens.js";
import type { AgentConfig as Config } from "./config.js";
import type { VerifiedCandidate } from "./findings.js";
import type { Sample } from "./evidence.js";

const config = {
  apiUrl: "http://127.0.0.1:4100",
  publicUrl: "https://lens.example.com",
  lensKey: "secret",
  agent: "selected",
} as Config;
const signal = AbortSignal.timeout(10_000);
const response = (data: unknown) => new Response(JSON.stringify(data));
const trace = (agent: string) => ({
  trace_id: agent,
  trace_ref: "ref",
  agent_names: [agent],
  start_time: "2026-01-01",
  duration_ms: 100,
  status: "error",
  llm_calls: 2,
  tool_calls: 1,
  error_count: 1,
  input_tokens: 50,
  output_tokens: 20,
  input_preview: "private prompt",
  api_key_hash: "private key",
});

test("cold trace reads get a bounded longer deadline while caller cancellation still wins", async (context) => {
  const timeouts: number[] = [];
  context.mock.method(AbortSignal, "timeout", (milliseconds: number) => {
    timeouts.push(milliseconds);
    return new AbortController().signal;
  });
  const parent = new AbortController();
  let active: AbortSignal | null | undefined;
  const client = new LensClient(config, async (_url, init) => {
    active = init?.signal;
    return response({});
  });
  await client.get("/v1/traces?start_ms=0", parent.signal);
  assert.deepEqual(timeouts, [60_000]);
  assert.equal(active?.aborted, false);
  parent.abort();
  assert.equal(active?.aborted, true);
  await client.get("/lens", new AbortController().signal);
  assert.deepEqual(timeouts, [60_000, 10_000]);
});

test("bounded trace scans select exact agent and remove captured content", async () => {
  let requests = 0;
  const client = new LensClient(config, async (_url, init) => {
    requests++;
    assert.equal(new Headers(init?.headers).get("X-Lens-Contract"), null);
    assert.equal(init?.redirect, "error");
    return response({
      data: [trace("selected"), trace("other")],
      next_cursor: "more",
    });
  });
  const value = await client.recent(signal);
  const result = JSON.stringify(value);
  assert.equal(requests, 100);
  assert.equal(value.traces.length, 1);
  assert.equal(value.incomplete, true);
  assert(
    value.traces.every(
      (item: { trace_id: string }) => item.trace_id === "selected",
    ),
  );
  assert(!result.includes("private"));
});

test("a lookback fixes one end across pagination while omitted windows retain the historical scan", async (context) => {
  const now = Date.parse("2026-01-02T12:00:00Z");
  context.mock.timers.enable({ apis: ["Date"], now });
  const requests: URL[] = [];
  const client = new LensClient(config, async (url) => {
    requests.push(new URL(String(url)));
    context.mock.timers.tick(60_000);
    return response({
      data: [],
      next_cursor: requests.length === 1 ? "page-two" : null,
    });
  });
  const selected = await client.recent(signal, 15, { lookback_hours: 12 });
  assert.equal(requests.length, 2);
  for (const request of requests) {
    assert.equal(
      request.searchParams.get("start_ms"),
      String(now - 43_200_000),
    );
    assert.equal(request.searchParams.get("end_ms"), String(now));
  }
  assert.equal(requests[1]?.searchParams.get("cursor"), "page-two");
  assert.equal(
    selected.window,
    "Traces started from 2026-01-02T00:00:00.000Z (inclusive) to 2026-01-02T12:00:00.000Z (exclusive)",
  );
  const retained = await client.recent(signal);
  assert.equal(requests[2]?.searchParams.get("start_ms"), "0");
  assert.equal(retained.window, "all retained history");
});

test("window boundaries and immutable agent scope exclude unrelated traces before report denominators", async () => {
  const start = "2026-01-02T00:00:00Z";
  const end = "2026-01-02T12:00:00Z";
  const rows = [
    ["before", "2026-01-01T23:59:59.999Z"],
    ["at-start", start],
    ["inside", "2026-01-02T11:59:59.999Z"],
    ["at-end", end],
    ["after", "2026-01-02T12:00:00.001Z"],
  ].map(([id, time]) => ({
    ...trace("selected"),
    trace_id: id!,
    start_time: time!,
  }));
  const details: string[] = [];
  const client = new LensClient(config, async (url) => {
    const target = new URL(String(url));
    if (target.pathname === "/v1/traces") {
      assert.equal(
        target.searchParams.get("start_ms"),
        String(Date.parse(start)),
      );
      assert.equal(target.searchParams.get("end_ms"), String(Date.parse(end)));
      return response({
        data: [...rows, { ...trace("other"), start_time: start }],
        next_cursor: null,
      });
    }
    const id = target.pathname.split("/")[3]!;
    if (target.pathname.includes("/spans/"))
      return response({
        span_id: "root",
        input: JSON.stringify([
          { role: "user", content: "Please finish this task" },
        ]),
        output: "Task result",
        attributes: { "gen_ai.operation.name": "invoke_agent" },
      });
    details.push(id);
    return response({
      summary: { trace_id: id, agent_names: ["selected"] },
      spans: [
        {
          span_id: "root",
          parent_span_id: null,
          name: "selected",
          type: "agent",
          status: id === "at-start" ? "error" : "ok",
          duration_ms: 100,
          error: null,
        },
      ],
    });
  });
  const report = await client.report(signal, { start, end });
  assert.deepEqual(details.sort(), ["at-start", "inside"]);
  assert.equal(report.population.inspected_traces, 2);
  assert.equal(report.population.terminal_traces, 2);
  assert.deepEqual(
    report.metrics.find((metric) => metric.id === "root_errors"),
    {
      id: "root_errors",
      category: "Reliability",
      count: 1,
      total: 2,
      label: "1/2 completed interactive turns ended with root errors",
      title: "Interactive turns end with an error status",
      unit: "completed interactive turns",
      affected_trace_ids: ["at-start"],
    },
  );
  assert.equal(
    report.window,
    "Traces started from 2026-01-02T00:00:00.000Z (inclusive) to 2026-01-02T12:00:00.000Z (exclusive)",
  );
});

test("invalid windows fail closed before any HTTP request or agent scope override", async () => {
  const end = "2026-01-02T12:00:00Z";
  let requests = 0;
  const client = new LensClient(config, async () => {
    requests++;
    return response({ data: [], next_cursor: null });
  });
  for (const window of [
    { lookback_hours: 0 },
    { lookback_hours: -1 },
    { lookback_hours: Number.POSITIVE_INFINITY },
    { start: "2026-01-02T00:00:00Z" },
    { end },
    { start: end, end },
    { start: "2026-01-03T00:00:00Z", end },
    { start: "2026-01-01", end },
    { start: "2026-01-01T12:00:00", end },
    { lookback_hours: 12, start: "2026-01-02T00:00:00Z", end },
    {
      lookback_hours: 12,
      end: new Date(Date.now() + 86_400_000).toISOString(),
    },
    { lookback_hours: 12, agent: "other" },
  ]) {
    const result = JSON.parse(
      await client.read("recent_traces", signal, window),
    );
    assert(result.unavailable, JSON.stringify(window));
  }
  assert.equal(requests, 0);
});

test("findings preserve candidate uncertainty and canonical evidence route without raw quotes", async () => {
  const finding = {
    id: "f1",
    title: "Wrong tool",
    description: "Argument mismatch",
    kind: "issue",
    status: "open",
    last_seen: "2026-01-01",
    evidence: [
      { execution_id: "run1", role: "support", quote: "raw captured prompt" },
    ],
  };
  const client = new LensClient(config, async () =>
    response({
      lenses: [
        { id: "l1", settings: { agent_name: "selected" }, findings: [finding] },
        {
          id: "l2",
          settings: { agent_name: "other" },
          findings: [{ ...finding, title: "Other private issue" }],
        },
      ],
    }),
  );
  const raw = await client.read("findings", signal);
  const result = JSON.parse(raw);
  assert.equal(result.findings.length, 1);
  assert.equal(result.findings[0].supporting_sampled_runs, 1);
  assert.equal(
    new URL(result.findings[0].url).searchParams.get("issue"),
    "l1:f1",
  );
  assert(result.warning.includes("Unvalidated"));
  assert(!raw.includes("raw captured") && !raw.includes("Other private"));
});

test("eval counts do not expose unknown cost or claim a paired gain", async () => {
  const client = new LensClient(config, async (url, init) => {
    assert.equal(new Headers(init?.headers).get("X-Lens-Contract"), "2");
    assert.equal(new URL(String(url)).searchParams.get("agent"), config.agent);
    return response([
      {
        id: "r1",
        agent: "selected",
        eval: "quality",
        version: "commit",
        status: "done",
        expected_trials: 10,
        received_trials: 10,
        summary: {
          passed: 8,
          total: 10,
          errors: 0,
          baseline_run_id: "baseline",
          baseline_version: "older",
          scores: { task_completed: 0.8 },
          cost_per_case: 0,
          regressions: [],
          fixed: [],
          gate: { passed: true },
        },
      },
    ]);
  });
  const raw = await client.read("eval_results", signal);
  const result = JSON.parse(raw);
  assert.equal(result.runs[0].summary.passed, 8);
  assert.equal(result.runs[0].summary.total, 10);
  assert(result.warning.includes("not a verified paired experiment"));
  assert(!raw.includes("cost_per_case"));
});

test("invalid or failed evidence reads return unknown without leaking response bodies", async () => {
  for (const reply of [
    new Response("secret failure", { status: 403 }),
    response({ invalid: true }),
    new Response("x".repeat(1_048_577)),
  ]) {
    const result = await new LensClient(config, async () => reply).read(
      "findings",
      signal,
    );
    assert(result.includes("unavailable"));
    assert(!result.includes("secret failure"));
  }
});

const imported: VerifiedCandidate = {
  fingerprint: "a".repeat(64),
  evidenceHash: "evidence",
  evidenceLinks: ["https://lens.example.com/?trace=selected"],
  codeLinks: ["https://github.com/example/repo/blob/sha/tool.ts#L1"],
  frequency: {
    count: 2,
    total: 3,
    unit: "tool calls",
    label: "2/3 tool calls failed",
    title: "tool failed",
  },
  candidate: {
    kind: "issue",
    category: "Reliability",
    confidence: 0.9,
    impact: 8,
    frequency_metric: "tool_error",
    issue_key: "tool_error",
    title: "tool failed",
    observation: "The tool returned an error",
    code_hypothesis: "The route may be unavailable",
    experiment: "Replay the affected cases and compare outcomes",
    outcome: "users could complete the task",
    limitation: "No validated improvement",
    evidence: [
      { trace_id: "selected", span_id: "tool", quote: "HTTP 503 unavailable" },
    ],
    code: [{ path: "tool.ts", quote: "callTool()" }],
  },
};
const importSample = {
  traces: [
    {
      ...trace("selected"),
      service: "selected",
      url: "https://lens.example.com/?trace=selected",
    },
  ],
} satisfies Pick<Sample, "traces">;
const importSource = {
  model: "model",
  prompt_revision: "version",
  trace_ids: ["selected"],
  repository_sha: "sha",
  at: 0,
};
const nativeLens = (id: string, agent = "selected") => ({
  id,
  settings: { agent_name: agent },
  findings: [],
});

test("native publication sends scoped exact evidence and returns the server's canonical finding identity", async () => {
  const sent: { url: string; body: Record<string, unknown> }[] = [];
  const client = new LensClient(
    { ...config, findingLensId: "chosen" },
    async (url, init) => {
      if (init?.method === "GET")
        return response({
          lenses: [nativeLens("other"), nativeLens("chosen")],
        });
      assert.equal(
        new Headers(init?.headers).get("Content-Type"),
        "application/json",
      );
      sent.push({ url: String(url), body: JSON.parse(String(init?.body)) });
      return response({
        lens_id: "chosen",
        finding_id: "consolidated-canonical-id",
      });
    },
  );
  const result = await client.persistFinding(
    imported,
    importSample,
    importSource,
    signal,
  );
  assert.equal(sent[0]?.url, config.apiUrl + "/lens/chosen/findings/import");
  assert.deepEqual(sent[0]?.body.evidence, [
    { ...imported.candidate.evidence[0], trace_ref: "ref" },
  ]);
  assert.equal(sent[0]?.body.fingerprint, imported.fingerprint);
  assert(String(sent[0]?.body.description).includes("Impact: 8/10"));
  assert(
    String(sent[0]?.body.description).includes("Customer benefit if validated"),
  );
  assert(
    String(sent[0]?.body.limitation).includes("No measured improvement yet"),
  );
  assert.equal(result.findingId, "consolidated-canonical-id");
  assert.equal(
    new URL(result.url).searchParams.get("issue"),
    "chosen:consolidated-canonical-id",
  );
});

test("native publication fails closed for ambiguous targets, wrong agents, absent trace refs and mismatched responses", async () => {
  for (const targets of [
    [],
    [nativeLens("one"), nativeLens("two")],
    [nativeLens("wrong", "other")],
  ]) {
    let posts = 0;
    const client = new LensClient(config, async (_url, init) => {
      if (init?.method === "POST") posts++;
      return response({ lenses: targets });
    });
    await assert.rejects(
      client.persistFinding(imported, importSample, importSource, signal),
      /matching native Lens/,
    );
    assert.equal(posts, 0);
  }
  const client = new LensClient(config, async (_url, init) =>
    response(
      init?.method === "POST"
        ? { lens_id: "wrong", finding_id: "id" }
        : { lenses: [nativeLens("one")] },
    ),
  );
  await assert.rejects(
    client.persistFinding(
      imported,
      {
        ...importSample,
        traces: [{ ...importSample.traces[0]!, trace_ref: "" }],
      },
      importSource,
      signal,
    ),
    /exact trace reference/,
  );
  await assert.rejects(
    client.persistFinding(imported, importSample, importSource, signal),
    /identity does not match/,
  );
});
