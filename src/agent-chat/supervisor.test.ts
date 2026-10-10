import assert from "node:assert/strict";
import { test } from "node:test";
import { setImmediate } from "node:timers/promises";
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

test("repeated sidecar exits keep retrying with capped backoff until shutdown", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  const warnings = context.mock.method(console, "warn", () => {});
  const completed = supervise(
    node("setInterval(() => {}, 1000)"),
    node("process.exit(3)"),
    1000,
  );
  const waitForExit = async (count: number) => {
    const deadline = performance.now() + 3000;
    while (warnings.mock.callCount() < count) {
      assert(performance.now() < deadline, "sidecar did not exit");
      await setImmediate();
    }
    assert.equal(warnings.mock.callCount(), count);
  };
  try {
    for (let attempt = 1; attempt <= 11; attempt++) {
      await waitForExit(attempt);
      context.mock.timers.tick(Math.min(attempt, 10) * 1000 - 1);
      await setImmediate();
      assert.equal(warnings.mock.callCount(), attempt);
      context.mock.timers.tick(1);
    }
    await waitForExit(12);
    process.emit("SIGTERM");
    assert.equal(await completed, 143);
    context.mock.timers.tick(1_000_000);
    await setImmediate();
    assert.equal(warnings.mock.callCount(), 12);
  } finally {
    process.emit("SIGTERM");
    await completed;
  }
});
