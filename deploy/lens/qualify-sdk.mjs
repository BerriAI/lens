import assert from "node:assert/strict";
import { createHash, randomBytes } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { execute, privateValue, root, tracePayload, withFixture } from "./qualification-fixture.mjs";

const wheel = path.resolve(process.argv[3]);
const digest = createHash("sha256").update(await fs.readFile(wheel)).digest("hex");
await withFixture("sdk", process.argv[2], async (fixture) => {
  await fixture.startServer();
  const { request } = fixture;
  const key = await request("POST", "/lens/tracing/keys", { name: "Packaged SDK qualification" });
  assert.equal(key.status, 200);
  const token = privateValue(key.data.key);
  const traces = [];
  for (const tools of [["run_tests", "publish"], ["publish", "run_tests"]]) {
    const trace = tracePayload();
    trace.resourceSpans[0].resource.attributes.push(
      { key: "agent.name", value: { stringValue: "release-qualification" } },
      { key: "agent.version", value: { stringValue: traces.length === 0 ? "baseline" : "candidate" } },
      { key: "deployment.environment", value: { stringValue: "lens-eval" } },
    );
    const spans = trace.resourceSpans[0].scopeSpans[0].spans;
    const parent = spans[0];
    const start = BigInt(parent.startTimeUnixNano);
    parent.endTimeUnixNano = String(start + 1000000000n);
    for (const [index, name] of tools.entries()) spans.push({
      traceId: parent.traceId, spanId: randomBytes(8).toString("hex"), parentSpanId: parent.spanId, name,
      startTimeUnixNano: String(start + BigInt(index + 1) * 1000000n),
      endTimeUnixNano: String(start + BigInt(index + 2) * 1000000n), status: { code: 1 },
      attributes: [{ key: "gen_ai.operation.name", value: { stringValue: "execute_tool" } },
        { key: "gen_ai.tool.name", value: { stringValue: name } }],
    });
    assert.equal((await request("POST", "/v1/traces", trace, token)).status, 200);
    traces.push(parent.traceId);
  }
  const dataset = await request("POST", "/lens/datasets", { name: "Packaged SDK", agent_name: "release-qualification" });
  assert.equal(dataset.status, 200);
  const saved = await request("POST", `/lens/datasets/${dataset.data.id}/revisions`, {
    base_revision: 0,
    cases: [{ id: "release-case", messages: [{ role: "user", content: "Qualify the packaged SDK" }],
      reply: "", tool_calls: [], expected: "Tests precede publication", included: true,
      source: { trace_id: traces[0], trace_ref: "", span_id: "", finding_id: "", lens_id: "" }, agent_version: "baseline" }],
  });
  assert.equal(saved.status, 200);
  assert.equal(saved.data.revision, 1);
  const latest = await request("POST", `/lens/datasets/${dataset.data.id}/revisions`, {
    base_revision: 1, cases: saved.data.cases.map((item) => ({ ...item, expected: "This is revision two" })),
  });
  assert.equal(latest.status, 200);
  assert.equal(latest.data.revision, 2);
  const environment = path.join(fixture.directory, "wheel-env");
  await execute(process.env.LENS_QUALIFICATION_PYTHON ?? "python3", ["-m", "venv", environment]);
  const python = path.join(environment, "bin/python");
  await execute(python, ["-m", "pip", "install", "--disable-pip-version-check", wheel]);
  const result = JSON.parse(await execute(python, [path.join(root, "src/sdk/tests/runtime_smoke.py")], {
    timeout: 300000,
    environment: { PYTHONPATH: "", LENS_API_KEY: fixture.admin, LENS_BASE_URL: fixture.origin,
      LENS_SMOKE_WHEEL_SHA256: digest, LENS_SMOKE_GOOD_TRACE: traces[0], LENS_SMOKE_BAD_TRACE: traces[1] },
  }));
  const denied = await fetch(fixture.origin + "/lens/evals/runs", {
    headers: { authorization: `Bearer ${token}`, "X-Lens-Contract": "1" },
  });
  assert.equal(denied.status, 401);
  return { wheel: path.basename(wheel), wheel_sha256: digest, ...result, ingestion_key_cannot_read_evals: true, providers_contacted: false };
});
