import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
export const postgresImage = "postgres:16-alpine@sha256:721873c34ceb9f8d8fc265984940dc982404c105f19ad51be9fdc5970a6080ea";
const clickhouseImage = "mirror.gcr.io/clickhouse/clickhouse-server:26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e";
const secrets = new Set();
export const privateValue = (value = randomBytes(32).toString("hex")) => { secrets.add(value); return value; };
const redact = (text) => [...secrets].reduce((result, secret) => result.replaceAll(secret, "[redacted]"), text);

export async function execute(command, args, { input, environment = {}, expected = 0, timeout = 180000 } = {}) {
  const child = spawn(command, args, { env: { ...process.env, ...environment }, stdio: ["pipe", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.on("data", (data) => { stdout += data; });
  child.stderr.on("data", (data) => { stderr += data; });
  child.stdin.on("error", () => {});
  child.stdin.end(input);
  const timer = setTimeout(() => child.kill("SIGKILL"), timeout);
  const status = await new Promise((resolve, reject) => {
    child.once("error", reject);
    child.once("close", resolve);
  }).finally(() => clearTimeout(timer));
  const diagnostic = `${path.basename(command)} exited ${status}: ${redact(stderr.slice(-2000))}`;
  if (expected === "failure") assert.ok(status !== 0 && status !== null, diagnostic);
  else assert.equal(status, expected, diagnostic);
  return stdout.trim();
}

export async function waitFor(check, label) {
  for (let attempt = 0; attempt < 120; attempt++) {
    if (await check().catch(() => false)) return;
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  assert.fail(`Timed out waiting for ${label}`);
}

export async function withFixture(lane, image, run) {
  assert.ok(image, "Pass a locally built candidate image");
  const inspected = JSON.parse(await execute("docker", ["image", "inspect", image]))[0];
  const directory = await fs.mkdtemp(path.join(os.tmpdir(), `lens-${lane}-`));
  await fs.chmod(directory, 0o700);
  const name = `lens-${lane}-${process.pid}-${randomBytes(4).toString("hex")}`;
  const containers = [];
  const admin = privateValue();
  const password = privateValue();
  const environmentFile = path.join(directory, "runtime.env");
  const storage = `${name}-clickhouse`;
  const server = `${name}-server`;
  await fs.writeFile(environmentFile, `LENS_MODE=standalone\nLENS_ADMIN_TOKEN=${admin}\nCLICKHOUSE_HOST=${storage}\nCLICKHOUSE_DATABASE=lens\nCLICKHOUSE_DB=lens\nCLICKHOUSE_USER=lens\nCLICKHOUSE_PASSWORD=${password}\nCLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT=1\n`, { mode: 0o600 });
  const docker = (args, options) => execute("docker", args, options);
  const networks = [];
  let cleaning;
  const cleanup = () => cleaning ??= (async () => {
    for (const id of containers.toReversed()) await docker(["rm", "--force", "--volumes", id]).catch(() => {});
    for (const network of networks.toReversed()) await docker(["network", "rm", network]).catch(() => {});
    await fs.rm(directory, { recursive: true, force: true });
  })();
  const interrupt = () => { void cleanup().finally(() => process.exit(130)); };
  process.once("SIGINT", interrupt);
  process.once("SIGTERM", interrupt);
  const create = async (suffix, args, network = name) => {
    const container = `${name}-${suffix}`;
    const id = await docker(["create", "--name", container, "--network", network, ...args]);
    containers.push(id);
    return container;
  };
  const sql = (query) => docker(["exec", storage, "sh", "-c", 'exec clickhouse-client --user lens --password "$CLICKHOUSE_PASSWORD" --query "$1"', "sh", query]);
  const waitStorage = () => waitFor(async () => (await sql("SELECT 1")) === "1", "ClickHouse");
  let origin;
  const request = async (method, endpoint, body, credential = admin) => {
    const response = await fetch(origin + endpoint, {
      method, signal: AbortSignal.timeout(45000),
      headers: { "content-type": "application/json", ...(credential ? { authorization: `Bearer ${credential}` } : {}) },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await response.text();
    let data;
    try { data = JSON.parse(text); } catch { data = text; }
    return { status: response.status, data };
  };
  const waitServer = async () => {
    const port = await docker(["port", server, "4318/tcp"]);
    assert.match(port, /^127\.0\.0\.1:\d+$/);
    origin = `http://${port}`;
    await waitFor(async () => (await request("GET", "/health/ready")).status === 200, "Lens readiness");
  };
  try {
    await docker(["network", "create", "--internal", name]);
    networks.push(name);
    await create("clickhouse", ["--memory", "2g", "--cpus", "2", "--env-file", environmentFile, clickhouseImage]);
    await docker(["cp", path.join(root, "deploy/clickhouse/keeper.xml"), `${storage}:/etc/clickhouse-server/config.d/lens-keeper.xml`]);
    await docker(["start", storage]);
    await waitStorage();
    const startServer = async () => {
      const apiNetwork = `${name}-api`;
      await docker(["network", "create", apiNetwork]);
      networks.push(apiNetwork);
      await create("server", ["--read-only", "--cap-drop", "ALL", "--security-opt", "no-new-privileges", "--pids-limit", "128", "--memory", "2g", "--cpus", "2", "--tmpfs", "/tmp:rw,noexec,nosuid,size=1g", "--env-file", environmentFile, "--publish", "127.0.0.1::4318", inspected.Id], apiNetwork);
      await docker(["network", "connect", name, server]);
      await docker(["start", server]);
      await waitServer();
    };
    const result = await run({ directory, name, image: inspected.Id, admin, password, environmentFile, storage, server, docker, create, sql, waitStorage, startServer, waitServer, request, get origin() { return origin; } });
    console.log(JSON.stringify({ status: "passed", lane, image: inspected.Id, architecture: inspected.Architecture,
      runtime_source_sha: inspected.Config.Labels?.["org.opencontainers.image.revision"] ?? null,
      harness_source_sha: await execute("git", ["-C", root, "rev-parse", "HEAD"]), ...result }));
  } finally {
    await cleanup();
    process.removeListener("SIGINT", interrupt);
    process.removeListener("SIGTERM", interrupt);
  }
}

export function tracePayload(traceId = randomBytes(16).toString("hex")) {
  const now = BigInt(Date.now()) * 1000000n;
  return { resourceSpans: [{ resource: { attributes: [{ key: "service.name", value: { stringValue: "release-qualification" } }] },
    scopeSpans: [{ spans: [{ traceId, spanId: randomBytes(8).toString("hex"), name: "Release qualification",
      startTimeUnixNano: String(now), endTimeUnixNano: String(now + 1000000n), status: { code: 1 },
      attributes: [{ key: "gen_ai.operation.name", value: { stringValue: "invoke_agent" } }, { key: "gen_ai.agent.name", value: { stringValue: "release-qualification" } }] }] }] }] };
}
