import assert from "node:assert/strict";
import { test } from "node:test";
import { supervise } from "./supervisor.js";

const node = (code: string) => ({ file: process.execPath, args: ["-e", code] });

test("server CLI arguments and exit status survive supervision", async () => {
  const status = await supervise({
    file: process.execPath,
    args: [
      "-e",
      "process.exit(process.argv[1] === 'expected' ? 7 : 9)",
      "expected",
    ],
  });
  assert.equal(status, 7);
});

test("a sidecar failure never fails the server", async () => {
  assert.equal(
    await supervise(
      node("setTimeout(() => process.exit(0), 100)"),
      node("process.exit(3)"),
      10,
    ),
    0,
  );
});

test("server termination stops its long-running sidecar", async () => {
  const start = Date.now();
  assert.equal(
    await supervise(
      node("setTimeout(() => process.exit(2), 60)"),
      node("setInterval(() => {}, 1000)"),
    ),
    2,
  );
  assert(Date.now() - start < 3000);
});
