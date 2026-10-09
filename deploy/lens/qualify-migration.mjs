import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import fs from "node:fs/promises";
import path from "node:path";
import { postgresImage, privateValue, root, tracePayload, waitFor, withFixture } from "./qualification-fixture.mjs";

await withFixture("migration", process.argv[2], async (fixture) => {
  const { docker, directory } = fixture;
  const postgresEnvironment = path.join(directory, "postgres.env");
  await fs.writeFile(postgresEnvironment, `POSTGRES_PASSWORD=${privateValue()}\n`, { mode: 0o600 });
  const postgres = await fixture.create("postgres", ["--memory", "512m", "--cpus", "1", "--env-file", postgresEnvironment, postgresImage]);
  await docker(["start", postgres]);
  await waitFor(async () => { await docker(["exec", postgres, "pg_isready", "-h", "127.0.0.1", "-U", "postgres"]); return true; }, "PostgreSQL fixture");
  const sql = (input) => docker(["exec", "-i", postgres, "psql", "-U", "postgres", "--no-psqlrc", "--tuples-only", "--no-align", "--set=ON_ERROR_STOP=on"], { input });
  await sql(await fs.readFile(path.join(root, "src/worker/crates/migrate/tests/support/postgres.sql"), "utf8"));
  const publicFixture = JSON.parse(await fs.readFile(path.join(root, "src/worker/crates/contract/tests/fixtures/investigations_public.json"), "utf8"));
  const ingestionKey = privateValue();
  const ingestionHash = createHash("sha256").update(ingestionKey).digest("hex");
  const dataset = { id: "dataset", name: "Migration qualification", agent_name: "agent", team_id: "alpha", created_at: "2026-01-15T00:00:00Z", revision: 1, created_by: "owner", cases: [
    { id: "saved-case", messages: [{ role: "user", content: "Find order 42", name: "", tool_calls: [] }], reply: "Found order 42", tool_calls: [{ name: "lookup", arguments: "{\"order_id\":42}" }], expected: "Order found", included: false,
      source: { trace_id: "trace", trace_ref: "backend-key", span_id: "span", finding_id: "finding", lens_id: "lens" }, agent_version: "v1" },
  ] };
  const source = {
    lenses: [{ id: "lens", version: 42, data: { ...publicFixture.filled_lens, settings: { ...publicFixture.filled_lens.settings, enabled: false } }, due_at: null }],
    runs: [{ id: "archive", lens_id: "lens", created_at: "2026-01-15T00:00:00", data: { ...publicFixture.job, id: "archive" } }],
    reviews: [{ lens_id: "lens", criteria_key: "retained-review-key", execution_id: publicFixture.review.execution_id, data: publicFixture.review }],
    workers: [{ id: publicFixture.worker.id, token_hash: "retained-worker-hash", data: publicFixture.worker }],
    ingestion_keys: [{ id: "migrated-key", data: { id: "migrated-key", name: "Migrated tracing", tenant: { team_id: "alpha", user_id: "owner", org_id: "organization", api_key_hash: ingestionHash }, created_at: "2026-01-15T00:00:00Z", expires_at: Math.floor(Date.now() / 1000) + 3600 } }],
    datasets: [1, 2].map((revision) => ({ id: "dataset", revision, created_at: `2026-01-1${revision + 5}T00:00:00`, data: { ...dataset, revision } })),
    signal_configs: [{ id: "global", data: {} }],
    trace_signals: [{ trace_id: "trace", trace_ref: "backend-key", config_key: "retained-config-key", span_count: 3, claimed_until: "2026-01-15T00:00:00", classified_at: null, data: { status: "pending", scores: {}, model: "signals", error: "" } }],
  };
  const tables = { lenses: "LiteLLM_Lens", runs: "LiteLLM_LensRun", reviews: "LiteLLM_LensReview", workers: "LiteLLM_LensWorker", ingestion_keys: "LiteLLM_LensIngestionKey", datasets: "LiteLLM_LensDataset", signal_configs: "LiteLLM_LensSignalConfig", trace_signals: "LiteLLM_LensTraceSignal" };
  const literal = (value) => `'${JSON.stringify(value).replaceAll("'", "''")}'::jsonb`;
  for (const [group, table] of Object.entries(tables)) await sql(`INSERT INTO "${table}" SELECT * FROM jsonb_populate_recordset(NULL::"${table}", ${literal(source[group])});`);
  const snapshot = () => sql(Object.values(tables).map((table) => `SELECT coalesce(jsonb_agg(to_jsonb(row) ORDER BY to_jsonb(row)::text), '[]'::jsonb) FROM "${table}" AS row;`).join("\n"));
  const original = await snapshot();
  const migrationEnvironment = path.join(directory, "migration.env");
  await fs.writeFile(migrationEnvironment, `${await fs.readFile(fixture.environmentFile, "utf8")}LENS_MIGRATION_POSTGRES_URL=postgres://migration_reader:fixture-reader@${postgres}:5432/postgres\n`, { mode: 0o600 });
  const migrate = (args = [], expected = 0) => docker(["run", "--rm", "--network", fixture.name, "--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--pids-limit", "64", "--memory", "1g", "--cpus", "2", "--env-file", migrationEnvironment, "--entrypoint", "/usr/local/bin/lens-migrate", fixture.image, ...args], { expected });
  const emptyTarget = async () => assert.equal(await fixture.sql("SELECT count() FROM system.tables WHERE database='lens'"), "0", "Preflight must not write target tables");
  await migrate(["--apply"], "failure");
  await emptyTarget();
  await sql(`UPDATE "LiteLLM_LensIngestionKey" SET data=jsonb_set(data,'{tenant,api_key_hash}', '"malformed"'::jsonb);`);
  await migrate(["--apply", "--source-stopped"], "failure");
  await emptyTarget();
  await sql(`UPDATE "LiteLLM_LensIngestionKey" SET data=${literal(source.ingestion_keys[0].data)};`);
  await sql(`INSERT INTO "LiteLLM_LensIngestionKey" SELECT 'duplicate', jsonb_set(data,'{id}','"duplicate"'::jsonb) FROM "LiteLLM_LensIngestionKey";`);
  await migrate(["--apply", "--source-stopped"], "failure");
  await emptyTarget();
  await sql(`DELETE FROM "LiteLLM_LensIngestionKey" WHERE id='duplicate';`);
  assert.equal(await snapshot(), original);
  const planned = JSON.parse(await migrate());
  assert.equal(planned.applied, false);
  assert.equal(planned.verified, false);
  assert.deepEqual(planned.plan.source_rows, Object.fromEntries(Object.entries(source).map(([group, rows]) => [group, rows.length])));
  await emptyTarget();
  const applied = JSON.parse(await migrate(["--apply", "--source-stopped"]));
  assert.equal(applied.applied, true);
  assert.equal(applied.verified, true);
  assert.deepEqual(applied.plan, planned.plan);
  const heads = await fixture.sql("SELECT key,revision,digest FROM lens.lens_state_heads ORDER BY key FORMAT JSONEachRow");
  const blobs = await fixture.sql("SELECT key,revision,digest,data FROM lens.lens_state_blobs FINAL ORDER BY key,revision,digest FORMAT JSONEachRow");
  assert.deepEqual(JSON.parse(await migrate(["--apply", "--source-stopped"])), applied);
  assert.equal(await fixture.sql("SELECT key,revision,digest FROM lens.lens_state_heads ORDER BY key FORMAT JSONEachRow"), heads);
  assert.equal(await fixture.sql("SELECT key,revision,digest,data FROM lens.lens_state_blobs FINAL ORDER BY key,revision,digest FORMAT JSONEachRow"), blobs);
  assert.equal(await snapshot(), original, "The read-only source remains byte-identical");
  await docker(["rm", "--force", "--volumes", postgres]);
  await fixture.startServer();
  const read = async (endpoint) => {
    const result = await fixture.request("GET", endpoint);
    assert.equal(result.status, 200, endpoint);
    return result.data;
  };
  const lens = await read("/lens/lens");
  assert.deepEqual(lens.scope, source.lenses[0].data.scope);
  assert.equal(lens.findings.length, source.lenses[0].data.findings.length);
  assert.equal((await read("/lens/lens/runs/archive")).id, "archive");
  for (const revision of [1, 2]) {
    const value = await read(`/lens/datasets/dataset?revision=${revision}`);
    assert.equal(value.revision, revision);
    assert.equal(value.team_id, "alpha");
    assert.deepEqual(value.cases, dataset.cases);
  }
  assert.equal((await read("/lens/datasets/dataset")).revision, 2);
  assert.equal((await read("/lens/tracing/keys"))[0].tenant.api_key_hash, ingestionHash);
  assert.equal((await fixture.request("POST", "/v1/traces", tracePayload(), ingestionKey)).status, 200);
  assert.equal((await read("/v1/traces")).data.length, 1);
  return { source_groups: planned.plan.source_rows, records: planned.plan.records, source_read_only: true,
    invalid_sources_rejected_before_target_writes: 2, missing_handoff_flag_rejected: true, repeated_import_identical: true,
    imported_findings_and_dataset_revisions_readable: true, ingestion_hash_retained_and_usable: true,
    postgres_removed_before_runtime_start: true, providers_contacted: false };
});
