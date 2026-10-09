import assert from "node:assert/strict";
import { test } from "node:test";
import { LensClient } from "./lens.js";
import type { Config } from "./config.js";

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
