import assert from "node:assert/strict";
import { test } from "node:test";
import { setImmediate } from "node:timers/promises";
import { InvestigationClient } from "./model-client.js";

test("investigation requests wait without a model deadline and still honor cancellation", async (context) => {
  context.mock.timers.enable({ apis: ["setTimeout"] });
  let start!: (signal: AbortSignal) => void;
  const started = new Promise<AbortSignal>((resolve) => {
    start = resolve;
  });
  let finish!: (response: Response) => void;
  let fail!: (reason: unknown) => void;
  const response = new Promise<Response>((resolve, reject) => {
    finish = resolve;
    fail = reject;
  });
  const client = new InvestigationClient({
    apiKey: "fixture",
    maxRetries: 0,
    fetch: async (_url, init) => {
      assert(init?.signal);
      start(init.signal);
      init.signal.addEventListener("abort", () => fail(init.signal!.reason), {
        once: true,
      });
      return response;
    },
  });
  const request = client.responses.create({
    model: "fixture",
    input: "Analyze",
  });
  const result = Promise.resolve(request);
  const signal = await started;
  context.mock.timers.tick(3_600_000);
  await setImmediate();
  assert.equal(signal.aborted, false);
  finish(
    new Response(JSON.stringify({ id: "completed" }), {
      headers: { "content-type": "application/json" },
    }),
  );
  assert.equal((await result).id, "completed");

  const cancellation = new AbortController();
  let requestStarted!: () => void;
  const pending = new Promise<void>((resolve) => {
    requestStarted = resolve;
  });
  const cancelClient = new InvestigationClient({
    apiKey: "fixture",
    maxRetries: 0,
    fetch: async (_url, init) =>
      new Promise((_resolve, reject) => {
        assert(init?.signal);
        init.signal.addEventListener(
          "abort",
          () => reject(init.signal!.reason),
          { once: true },
        );
        requestStarted();
      }),
  });
  const canceled = assert.rejects(
    cancelClient.responses.create(
      { model: "fixture", input: "Analyze" },
      { signal: cancellation.signal },
    ),
    /abort/i,
  );
  await pending;
  cancellation.abort();
  await canceled;
});
