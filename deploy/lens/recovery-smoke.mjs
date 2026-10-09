import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs/promises";
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const image = process.argv[2];
assert.ok(image, "Pass the built Lens image reference");
const assets = path.dirname(fileURLToPath(import.meta.url));
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), "lens-recovery-"));
await fs.chmod(temporary, 0o700);
const prefix = `lens-recovery-${process.pid}`;
const deployments = ["source", "restored"].map((name) => ({
  directory: path.join(temporary, name, "deploy", "lens"),
  project: `${prefix}-${name}`,
}));

async function execute(command, args, environment = {}, expected = 0) {
  const child = spawn(command, args, {
    env: { ...process.env, ...environment },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  child.stdout.on("data", (data) => { output += data; });
  child.stderr.on("data", (data) => { output += data; });
  const status = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", resolve);
  });
  if (expected === "failure") assert.notEqual(status, 0, `${command} unexpectedly succeeded`);
  else assert.equal(status, expected, output);
  return output;
}

async function availablePort() {
  const server = net.createServer();
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const port = server.address().port;
  await new Promise((resolve) => server.close(resolve));
  return port;
}

function compose(deployment, ...args) {
  return execute("docker", ["compose", "--project-name", deployment.project,
    "--project-directory", deployment.directory,
    "--file", path.join(deployment.directory, "compose.yaml"), ...args], deployment.environment);
}

try {
  for (const deployment of deployments) {
    await fs.mkdir(deployment.directory, { recursive: true });
    await fs.mkdir(path.join(deployment.directory, "../clickhouse"));
    for (const name of ["start", "backup", "restore", "compose.yaml"]) {
      await fs.copyFile(path.join(assets, name), path.join(deployment.directory, name));
    }
    await fs.copyFile(path.join(assets, "../clickhouse/keeper.xml"), path.join(deployment.directory, "../clickhouse/keeper.xml"));
    deployment.origin = `http://127.0.0.1:${await availablePort()}`;
    deployment.environment = {
      COMPOSE_PROJECT_NAME: deployment.project,
      LENS_PORT: new URL(deployment.origin).port,
      LENS_PUBLIC_URL: deployment.origin,
    };
  }
  const [source, restored] = deployments;
  const start = path.join(source.directory, "start");
  await execute(start, [], { ...source.environment, LENS_IMAGE: image });
  const environmentFile = path.join(source.directory, ".env");
  const originalEnvironment = await fs.readFile(environmentFile, "utf8");
  assert.equal((await fs.stat(environmentFile)).mode & 0o777, 0o600);
  await execute(start, [], { ...source.environment, LENS_IMAGE: "" });
  assert.equal(await fs.readFile(environmentFile, "utf8"), originalEnvironment);
  const token = originalEnvironment.split("\n").find((line) => line.startsWith("LENS_ADMIN_TOKEN=")).split("=")[1];
  async function request(deployment, method, endpoint, body, credential = token) {
    const response = await fetch(deployment.origin + endpoint, {
      method, headers: { authorization: `Bearer ${credential}`, "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    assert.ok(response.ok, `${method} ${endpoint}: ${response.status}`);
    const text = await response.text();
    try { return JSON.parse(text); } catch { return text; }
  }
  const key = await request(source, "POST", "/lens/tracing/keys", { name: "Recovery qualification" });
  const traceId = randomBytes(16).toString("hex");
  const now = BigInt(Date.now()) * 1000000n;
  const messages = [["gen_ai.operation.name", "chat"], ["gen_ai.request.model", "fixture"],
    ["gen_ai.input.messages", JSON.stringify([{ role: "user", content: "Can I recover my saved data?" }])],
    ["gen_ai.output.messages", JSON.stringify([{ role: "assistant", content: "Verify the saved trace and dataset." }])]];
  await request(source, "POST", "/v1/traces", {
    resourceSpans: [{ resource: { attributes: [{ key: "service.name", value: { stringValue: "recovery-proof" } }] },
      scopeSpans: [{ spans: [{ traceId, spanId: randomBytes(8).toString("hex"), name: "Recovery proof",
        startTimeUnixNano: String(now), endTimeUnixNano: String(now + 1000000n), status: { code: 1 },
        attributes: messages.map(([key, value]) => ({ key, value: { stringValue: value } })) }] }] }],
  }, key.key);
  const dataset = await request(source, "POST", "/lens/datasets", { name: "Recovery qualification", agent_name: "recovery-proof" });
  const built = await request(source, "POST", "/lens/datasets/build", { dataset_id: dataset.id, sources: [{ kind: "trace", trace_id: traceId }] });
  assert.equal(built.cases.length, 1);
  const saved = await request(source, "POST", `/lens/datasets/${dataset.id}/revisions`, { base_revision: 0, cases: built.cases });
  const exported = await request(source, "GET", `/lens/datasets/${dataset.id}/export`);
  const login = await fetch(source.origin + "/auth/session", { method: "POST",
    headers: { "content-type": "application/json" }, body: JSON.stringify({ token }) });
  assert.equal(login.status, 200);
  const cookie = login.headers.get("set-cookie").split(";")[0];
  await compose(source, "stop", "lens");
  const compacted = await compose(source, "run", "--rm", "--no-deps", "lens", "compact-state");
  assert.match(compacted, /Compacted [1-9][0-9]* state records/);
  await compose(source, "up", "--detach", "--wait", "--wait-timeout", "180");
  assert.deepEqual(await request(source, "GET", `/lens/datasets/${dataset.id}/export`), exported);
  assert.equal((await fetch(source.origin + "/lens/datasets", { headers: { cookie } })).status, 200);
  const shim = path.join(temporary, "failed-image-save");
  await fs.mkdir(shim);
  const docker = (await execute("which", ["docker"])).trim();
  await fs.writeFile(path.join(shim, "docker"), `#!/usr/bin/env node\nimport {spawnSync} from 'node:child_process';\nconst args=process.argv.slice(2);\nif(args[0]==='image'&&args[1]==='save')process.exit(42);\nconst result=spawnSync(${JSON.stringify(docker)},args,{stdio:'inherit'});\nprocess.exit(result.status??1);\n`, { mode: 0o755 });
  const failedBackup = path.join(temporary, "failed-snapshot");
  await execute(path.join(source.directory, "backup"), [failedBackup], {
    ...source.environment, PATH: `${shim}${path.delimiter}${process.env.PATH}`,
  }, "failure");
  await compose(source, "up", "--detach", "--wait", "--wait-timeout", "180");
  assert.deepEqual(await request(source, "GET", `/lens/datasets/${dataset.id}`), saved);
  await assert.rejects(fs.access(path.join(failedBackup, "SHA256SUMS")), { code: "ENOENT" });
  const backup = path.join(temporary, "snapshot");
  await execute(path.join(source.directory, "backup"), [backup], source.environment);
  await compose(source, "up", "--detach", "--wait", "--wait-timeout", "180");
  assert.deepEqual(await request(source, "GET", `/lens/datasets/${dataset.id}`), saved);
  const restore = path.join(restored.directory, "restore");
  await execute(restore, [backup], { ...restored.environment, COMPOSE_PROJECT_NAME: source.project }, "failure");
  const targetEnvironment = path.join(restored.directory, ".env");
  await fs.symlink(environmentFile, targetEnvironment);
  await execute(restore, [backup], restored.environment, "failure");
  assert.equal(await fs.readFile(environmentFile, "utf8"), originalEnvironment);
  await fs.unlink(targetEnvironment);
  const sums = path.join(backup, "SHA256SUMS");
  const originalSums = await fs.readFile(sums);
  await fs.writeFile(sums, "0".repeat(64) + "  clickhouse.tar\n");
  await execute(restore, [backup], restored.environment, "failure");
  await fs.writeFile(sums, originalSums);
  await compose(source, "stop");
  await execute(restore, [backup], restored.environment);
  assert.deepEqual(await request(restored, "GET", `/lens/datasets/${dataset.id}`), saved);
  assert.deepEqual(await request(restored, "GET", `/lens/datasets/${dataset.id}/export`), exported);
  const traces = await request(restored, "GET", "/v1/traces");
  assert.equal(traces.data.length, 1);
  assert.equal(traces.data[0].trace_id, traceId);
  assert.ok(JSON.stringify(await request(restored, "GET", "/lens/tracing/keys")).includes(key.record.id));
  assert.equal((await fetch(restored.origin + "/lens/datasets", { headers: { cookie } })).status, 200);
  await execute(restore, [backup], restored.environment, "failure");
  console.log(JSON.stringify({ status: "passed", image, trace_retained: true, dataset_revision: saved.revision,
    export_identical: true, session_retained: true, ingestion_key_retained: true,
    repeated_setup_preserves_credentials: true, rejects_existing_project: true,
    rejects_existing_environment: true, rejects_symlink: true, rejects_corrupt_backup: true,
    failed_backup_resumes_services: true, state_compaction_preserves_export_and_session: true }));
} finally {
  for (const deployment of deployments) {
    await compose(deployment, "down", "--volumes", "--remove-orphans").catch((error) => console.error(error.message));
  }
  await fs.rm(temporary, { recursive: true, force: true });
}
