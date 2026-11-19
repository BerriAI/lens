import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const assets = path.dirname(fileURLToPath(import.meta.url));

async function start(t, { pull = "0", inspect = "0", buildStatus = "0", image = "", build = "", bundle = "", saved = "" } = {}) {
  const root = await fs.mkdtemp(path.join(os.homedir(), "lens-start-test-"));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const directory = path.join(root, "deploy/lens");
  const bin = path.join(root, "bin");
  await fs.mkdir(directory, { recursive: true });
  await fs.mkdir(bin);
  await fs.copyFile(path.join(assets, "start"), path.join(directory, "start"));
  await fs.writeFile(path.join(root, "VERSION"), "1.2.3\n");
  if (bundle) await fs.writeFile(path.join(directory, "image"), bundle);
  if (saved) await fs.writeFile(path.join(directory, ".env"), `LENS_IMAGE=${saved}\n`);
  await fs.writeFile(path.join(bin, "docker"), `#!/usr/bin/env bash
set -eu
printf '%s\\n' "$*" >> "$CALLS"
case "$1" in
  pull) exit "$PULL_STATUS" ;;
  image) exit "$INSPECT_STATUS" ;;
  build) exit "$BUILD_STATUS" ;;
  run) touch "$DEPLOYMENT/.env" ;;
  compose)
    case "$*" in
      *'config --images lens') printf '%s\\n' "$SAVED_IMAGE" ;;
      *'exec -T lens'*) printf '%s' 'http://localhost:4318' ;;
    esac
    ;;
esac
`, { mode: 0o755 });
  const result = spawnSync(path.join(directory, "start"), [], {
    encoding: "utf8",
    env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, LENS_IMAGE: image,
      LENS_BUILD: build, CALLS: path.join(root, "calls"), DEPLOYMENT: directory,
      PULL_STATUS: pull, INSPECT_STATUS: inspect, BUILD_STATUS: buildStatus, SAVED_IMAGE: saved },
  });
  return { ...result, calls: await fs.readFile(path.join(root, "calls"), "utf8"),
    environment: await fs.readFile(path.join(directory, ".env"), "utf8").catch(() => "") };
}

test("first start pulls the release version without building", async (t) => {
  const result = await start(t);
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.calls, /pull ghcr.io\/berriai\/lens:1\.2\.3/);
  assert.doesNotMatch(result.calls, /^build /m);
  assert.match(result.calls, /ghcr.io\/berriai\/lens:1\.2\.3 init \/setup/);
});

test("an unavailable release falls back to the local build", async (t) => {
  const result = await start(t, { pull: "1" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Release image unavailable; building Lens from source into lens:local/);
  assert.match(result.calls, /pull ghcr.io\/berriai\/lens:1\.2\.3/);
  assert.match(result.calls, /^build .*--tag lens:local /m);
  assert.match(result.calls, /lens:local init \/setup/);
});

test("source opt-in overrides explicit, saved and bundled images without pulling", async (t) => {
  const result = await start(t, { build: "1", image: "custom:image", saved: "saved:image", bundle: "bundle@sha256:abc" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.calls, /^build .*--tag lens:local /m);
  assert.doesNotMatch(result.calls, /^pull /m);
  assert.match(result.calls, /lens:local init \/setup/);
});

test("saved release images are preserved on restart", async (t) => {
  const result = await start(t, { saved: "saved:image" });
  assert.equal(result.status, 0, result.stderr);
  assert.doesNotMatch(result.calls, /^(pull|build|run) /m);
});

test("saved local images rebuild the checkout without pulling a release", async (t) => {
  const result = await start(t, { saved: "lens:local" });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.calls, /^build .*--tag lens:local /m);
  assert.doesNotMatch(result.calls, /^pull /m);
  assert.match(result.calls, /lens:local init \/setup/);
  assert.equal(result.environment, "LENS_IMAGE=lens:local\n");
});

test("explicit images and forced source builds replace the saved image", async (t) => {
  for (const options of [{ image: "custom:image" }, { build: "1" }]) {
    const result = await start(t, { ...options, saved: "saved:image" });
    assert.equal(result.status, 0, result.stderr);
    assert.equal(result.environment, `LENS_IMAGE=${options.image || "lens:local"}\n`);
  }
});

test("explicit and bundled images never fall back to source", async (t) => {
  for (const options of [{ image: "custom:image" }, { bundle: "bundle@sha256:abc" }]) {
    const result = await start(t, options);
    assert.equal(result.status, 0, result.stderr);
    assert.doesNotMatch(result.calls, /^(pull|build) /m);
    assert.match(result.calls, /(?:custom:image|bundle@sha256:abc) init \/setup/);
  }
});

test("explicit and bundled pull errors stop startup without building", async (t) => {
  for (const options of [{ image: "custom:image" }, { bundle: "bundle@sha256:abc" }]) {
    const result = await start(t, { ...options, inspect: "1", pull: "1" });
    assert.equal(result.status, 1);
    assert.match(result.calls, /^pull /m);
    assert.doesNotMatch(result.calls, /^(build|run) /m);
  }
});

test("a failed fallback build stops before initialization", async (t) => {
  const result = await start(t, { pull: "1", buildStatus: "1" });
  assert.equal(result.status, 1);
  assert.match(result.calls, /^build /m);
  assert.doesNotMatch(result.calls, /^run /m);
});
