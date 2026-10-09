import assert from "node:assert/strict";
import { privateValue, tracePayload, withFixture } from "./qualification-fixture.mjs";

await withFixture("runtime", process.argv[2], async (fixture) => {
  const { request, docker, storage, server } = fixture;
  await fixture.startServer();
  for (const endpoint of ["/lens", "/v1/traces", "/lens/tracing/keys", "/lens/datasets"]) {
    assert.equal((await request("GET", endpoint, undefined, "")).status, 401, `${endpoint} requires authentication`);
    assert.equal((await request("GET", endpoint, undefined, "invalid-fixture-token")).status, 401, `${endpoint} rejects an invalid token`);
  }
  const created = await request("POST", "/lens/tracing/keys", { name: "Revocation qualification" });
  assert.equal(created.status, 200);
  assert.equal(created.data.active, true);
  const key = privateValue(created.data.key);
  assert.equal((await request("GET", "/v1/traces", undefined, key)).status, 401, "An ingestion key cannot read traces");
  assert.equal((await request("POST", "/v1/traces", tracePayload())).status, 401, "The admin key is not an ingestion key");
  const retained = tracePayload();
  assert.equal((await request("POST", "/v1/traces", retained, key)).status, 200);
  const traces = await request("GET", "/v1/traces");
  assert.equal(traces.status, 200);
  assert.equal(traces.data.data.length, 1);
  assert.equal(traces.data.data[0].trace_id, retained.resourceSpans[0].scopeSpans[0].spans[0].traceId);
  await new Promise((resolve) => setTimeout(resolve, 1100));
  assert.equal((await request("DELETE", `/lens/tracing/keys/${created.data.record.id}`)).status, 200);
  assert.equal((await request("POST", "/v1/traces", tracePayload(), key)).status, 401, "Revoked keys cannot ingest");
  const active = await request("POST", "/lens/tracing/keys", { name: "Outage qualification" });
  assert.equal(active.status, 200);
  const activeKey = privateValue(active.data.key);
  const dataset = await request("POST", "/lens/datasets", { name: "Retained through outage", agent_name: "release-qualification" });
  assert.equal(dataset.status, 200);
  await docker(["stop", "--time", "10", storage]);
  for (const [method, endpoint, body, token] of [
    ["GET", "/v1/traces", undefined, fixture.admin],
    ["POST", "/v1/traces", tracePayload(), activeKey],
    ["POST", "/lens/datasets", { name: "Must not persist", agent_name: "release-qualification" }, fixture.admin],
  ]) {
    const response = await request(method, endpoint, body, token);
    assert.ok(response.status >= 500 && response.status < 600, `${method} ${endpoint} must fail closed during storage outage, got ${response.status}`);
  }
  assert.equal((await request("GET", "/lens/datasets", undefined, "")).status, 401);
  await docker(["start", storage]);
  await fixture.waitStorage();
  await docker(["restart", server]);
  await fixture.waitServer();
  assert.equal((await request("GET", "/v1/traces")).data.data.length, 1, "Failed ingestion must not appear after recovery");
  assert.deepEqual((await request("GET", `/lens/datasets/${dataset.data.id}`)).data, dataset.data);
  const datasets = await request("GET", "/lens/datasets");
  assert.equal(datasets.status, 200);
  assert.ok(!JSON.stringify(datasets.data).includes("Must not persist"));
  assert.equal((await request("POST", "/v1/traces", tracePayload(), key)).status, 401, "Revocation persists across restart");
  assert.equal((await request("POST", "/v1/traces", tracePayload(), activeKey)).status, 200);
  assert.equal((await request("GET", "/v1/traces")).data.data.length, 2);
  return { auth_denials: 10, revoked_key_denied_before_and_after_restart: true, storage_outage_failures: 3,
    rejected_writes_absent_after_recovery: true, prior_data_retained: true, ingestion_resumes: true,
    providers_contacted: false, gateway_or_postgres_required: false };
});
