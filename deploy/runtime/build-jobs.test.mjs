import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import { test } from "node:test";

const image = process.env.LENS_BUILDER_TEST_IMAGE || "lens:builder";
const dockerfile = fs.readFileSync(new URL("Dockerfile", import.meta.url), "utf8");
const build = dockerfile.slice(dockerfile.indexOf('RUN jobs="'), dockerfile.indexOf("\nRUN cc "))
  .replace(/^RUN /, "").replaceAll(/\\\n/g, "");

function run(jobs, memory) {
  return spawnSync("docker", ["run", "--rm", "--memory", memory,
    "--env", `LENS_BUILD_JOBS=${jobs}`, "--entrypoint", "sh", image, "-c",
    `cargo() { printf 'cargo-jobs=%s\\n' "$CARGO_BUILD_JOBS"; }; ${build}`], { encoding: "utf8" });
}

const resources = spawnSync("docker", ["run", "--rm", "--entrypoint", "sh", image,
  "-c", "nproc; awk '/MemTotal:/ {print $2}' /proc/meminfo"], { encoding: "utf8" });
assert.equal(resources.status, 0, resources.stderr);
const [cores, memoryKiB] = resources.stdout.trim().split(/\s+/).map(Number);

for (const [memory, budget] of [["4g", 2], ["8g", 2], ["12g", 3]]) {
  test(`automatic parallelism respects the ${memory} Docker memory limit`, () => {
    const result = run("auto", memory);
    assert.equal(result.status, 0, result.stderr);
    const expected = Math.min(cores, Math.max(2, Math.min(budget, Math.floor(memoryKiB / 4194304))));
    assert.match(result.stdout, new RegExp(`Building Rust with ${expected} Cargo jobs`));
    assert.equal(result.stdout.match(new RegExp(`cargo-jobs=${expected}\\n`, "g"))?.length, 2);
  });
}

test("an explicit job count overrides automatic selection", () => {
  const result = run("1", "4g");
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /Building Rust with 1 Cargo jobs/);
  assert.equal(result.stdout.match(/cargo-jobs=1\n/g)?.length, 2);
});

for (const jobs of ["", "0", "invalid"]) {
  test(`invalid job count ${JSON.stringify(jobs)} stops before Cargo runs`, () => {
    const result = run(jobs, "4g");
    assert.equal(result.status, 1);
    assert.match(result.stderr, /LENS_BUILD_JOBS must be auto or a positive integer/);
    assert.doesNotMatch(result.stdout, /cargo-jobs=/);
  });
}
