import assert from "node:assert/strict";
import { test } from "node:test";
import { postWithChart } from "./slack-transport.js";

const rejection = {
  code: "slack_webapi_platform_error",
  data: {
    ok: false,
    error: "invalid_attachments",
    response_metadata: {
      messages: [
        "invalid slack file [json-pointer:/attachments/0/blocks/2/slack_file.id/slack_file]",
      ],
    },
  },
};
const client = (postMessage: () => Promise<unknown>) =>
  ({ chat: { postMessage } }) as unknown as Parameters<typeof postWithChart>[0];
const message = { channel: "C123", text: "A report" };

test("only a definitive new-image rejection retries the same message with bounded backoff", async () => {
  const delays: number[] = [];
  let attempts = 0;
  const result = await postWithChart(
    client(async () => {
      if (++attempts < 3) throw rejection;
      return { ok: true, ts: "123" };
    }),
    message,
    async (ms) => {
      delays.push(ms);
    },
  );
  assert.equal(result.ts, "123");
  assert.equal(attempts, 3);
  assert.deepEqual(delays, [1000, 2000]);
  attempts = 0;
  delays.length = 0;
  await assert.rejects(
    postWithChart(
      client(async () => {
        attempts++;
        throw rejection;
      }),
      message,
      async (ms) => {
        delays.push(ms);
      },
    ),
  );
  assert.equal(attempts, 5);
  assert.deepEqual(delays, [1000, 2000, 4000, 8000]);
});

test("network errors and other invalid blocks cannot blindly duplicate a delivered message", async () => {
  for (const error of [
    new Error("connection reset"),
    { ...rejection, data: { ...rejection.data, error: "ratelimited" } },
    {
      ...rejection,
      data: {
        ...rejection.data,
        response_metadata: { messages: ["invalid block length"] },
      },
    },
  ]) {
    let attempts = 0;
    await assert.rejects(
      postWithChart(
        client(async () => {
          attempts++;
          throw error;
        }),
        message,
        async () => {
          assert.fail("must not retry");
        },
      ),
    );
    assert.equal(attempts, 1);
  }
});
