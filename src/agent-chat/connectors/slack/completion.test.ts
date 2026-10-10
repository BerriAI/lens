import assert from "node:assert/strict";
import { test } from "node:test";
import { Chat, type Activity, type Mention } from "./chat.js";
import type { ReplyAgent } from "./reply.js";
import type { Answer } from "./slack.js";
import { slackReplies } from "./responses.js";
import { failureAnswer, within } from "./completion.js";
import type { ThreadRegistry } from "./threads.js";

const now = 1_800_000_000_000;
const event: Mention = {
  id: "event",
  workspace: "TTEST",
  channel: "CTEST",
  user: "UUSER",
  text: "<@ULENS> Inspect recent traces",
  ts: "1800000000.000000",
  addressed: true,
};
const answer: Answer = {
  title: "Reliability",
  summary: "The observed call failed",
  sources: [],
};
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
function transport() {
  const posted: Record<string, unknown>[] = [];
  const updated: Record<string, unknown>[] = [];
  const slack = {
    chat: {
      postMessage: async (message: Record<string, unknown>) => {
        posted.push(message);
        return { ok: true, ts: "1800000000.123456" };
      },
      update: async (message: Record<string, unknown>) => {
        updated.push(message);
        return { ok: true, ts: message.ts };
      },
    },
    filesUploadV2: async () => {
      throw new Error("Unexpected chart upload");
    },
  } as unknown as Parameters<typeof slackReplies>[0];
  return { slack, posted, updated, ...slackReplies(slack) };
}
function create(
  reply: ReplyAgent,
  output: ReturnType<typeof transport>,
  activity: Activity = async () => {},
  timeoutMs = 1000,
) {
  return new Chat(event, reply, output.reply, () => now, undefined, activity, {
    acknowledge: output.acknowledge,
    timeoutMs,
    deliveryTimeoutMs: 100,
    cleanupTimeoutMs: 30,
  });
}

test("acknowledgment precedes delayed evidence and a stalled status call; the answer updates one message", async (t) => {
  const logs: string[] = [];
  t.mock.method(console, "info", (line: string) => logs.push(line));
  const model = deferred<Answer>();
  const output = transport();
  const states: string[] = [];
  let modelStarted = false;
  const chat = create(
    async () => {
      modelStarted = true;
      return model.promise;
    },
    output,
    async (_event, state) => {
      states.push(state);
      if (state === "working") await new Promise(() => {});
    },
  );
  const running = chat.mention(event);
  await tick();
  assert.equal(output.posted.length, 1);
  assert(String(output.posted[0]?.text).startsWith("On it"));
  assert.equal(output.updated.length, 0);
  assert.equal(modelStarted, true);
  model.resolve(answer);
  await within(running, 500);
  assert.equal(output.posted.length, 1);
  assert.equal(output.updated.length, 1);
  assert.equal(output.updated[0]?.ts, "1800000000.123456");
  assert.deepEqual(states, ["working", "done"]);
  await chat.mention(event);
  assert.equal(output.posted.length, 1);
  const timing = JSON.parse(logs[0]!);
  assert.deepEqual(Object.keys(timing).sort(), [
    "acknowledgment_ms",
    "duration_ms",
    "event",
    "outcome",
  ]);
  assert.equal(timing.outcome, "done");
  assert(timing.acknowledgment_ms <= timing.duration_ms);
  assert(
    !logs.join().includes(event.user) && !logs.join().includes(event.text),
  );
});

test("quota and evidence errors replace the acknowledgment with a safe failure", async () => {
  for (const error of [
    new Error(
      '429 litellm.RateLimitError: private payload {"error":{"type":"insufficient_quota","code":"credit_balance_exhausted"}}',
    ),
    Object.assign(new Error("private gateway payload and secret key"), {
      status: 429,
      error: { code: "insufficient_quota", type: "credit_balance_exhausted" },
    }),
    new Error("private captured trace failure"),
  ]) {
    const output = transport();
    const states: string[] = [];
    const chat = create(
      async () => {
        throw error;
      },
      output,
      async (_event, state) => {
        states.push(state);
      },
    );
    await chat.mention(event);
    assert.equal(output.posted.length, 1);
    assert.equal(output.updated.length, 1);
    assert.equal(output.updated[0]?.ts, "1800000000.123456");
    const rendered = JSON.stringify(output.updated);
    assert(!rendered.includes("private") && !rendered.includes("secret key"));
    assert(
      rendered.includes(
        error.message.includes("trace failure")
          ? "could not finish reading"
          : "no credits",
      ),
    );
    assert.deepEqual(states, ["working", "failed"]);
  }
  assert.equal(
    failureAnswer({ status: 429, code: "rate_limit_exceeded" }).title,
    "Evidence unavailable",
  );
  assert.equal(
    failureAnswer(new Error("429 rate limit exceeded")).title,
    "Evidence unavailable",
  );
  assert.equal(
    failureAnswer(new Error("insufficient_quota without status")).title,
    "Evidence unavailable",
  );
  assert.equal(
    failureAnswer({
      cause: {
        response: { data: { error: { code: "credit_balance_exhausted" } } },
      },
    }).title,
    "Analysis blocked",
  );
});

test("a model that ignores cancellation cannot leave a pending acknowledgment or publish a late answer", async () => {
  const model = deferred<Answer>();
  const output = transport();
  let signal: AbortSignal | undefined;
  const states: string[] = [];
  const chat = create(
    async (_history, current) => {
      signal = current;
      return model.promise;
    },
    output,
    async (_event, state) => {
      states.push(state);
    },
    20,
  );
  await within(chat.mention(event), 500);
  assert.equal(signal?.aborted, true);
  assert.equal(output.updated.length, 1);
  assert(JSON.stringify(output.updated).includes("Analysis timed out"));
  assert.deepEqual(states, ["working", "failed"]);
  model.resolve(answer);
  await tick();
  assert.equal(output.updated.length, 1);
  assert.equal(output.posted.length, 1);
});

test("shutdown aborts active evidence, updates its message, clears activity and ignores new requests", async () => {
  const output = transport();
  let signal: AbortSignal | undefined;
  const states: string[] = [];
  const chat = create(
    async (_history, current) => {
      signal = current;
      return new Promise(() => {});
    },
    output,
    async (_event, state) => {
      states.push(state);
    },
  );
  const running = chat.mention(event);
  await tick();
  await chat.shutdown(100);
  await within(running, 500);
  assert.equal(signal?.aborted, true);
  assert(JSON.stringify(output.updated).includes("Analysis interrupted"));
  assert.deepEqual(states, ["working", "failed"]);
  await chat.mention({ ...event, ts: "1800000001.000000" });
  assert.equal(output.posted.length, 1);
});

test("ambiguous acknowledgment failures do not start evidence or blindly create a second message", async () => {
  const output = transport();
  let calls = 0;
  let posts = 0;
  output.slack.chat.postMessage = async () => {
    posts++;
    throw new Error("ambiguous delivery");
  };
  const chat = create(async () => {
    calls++;
    return answer;
  }, output);
  await chat.mention(event);
  assert.equal(posts, 1);
  assert.equal(calls, 0);
  assert.equal(output.updated.length, 0);
});

test("failed cleanup cannot hold the worker busy forever", async () => {
  const output = transport();
  const chat = create(
    async () => answer,
    output,
    async (_event, state) => {
      if (state !== "working") await new Promise(() => {});
    },
  );
  await within(chat.mention(event), 500);
  await within(chat.mention({ ...event, ts: "1800000001.000000" }), 500);
  assert.equal(output.posted.length, 2);
  assert.equal(output.updated.length, 2);
});

test("a chart upload completing after its deadline cannot overwrite the failure message", async () => {
  const output = transport();
  const upload = deferred<{
    ok: true;
    files: { ok: true; files: { id: string }[] }[];
  }>();
  const uploadStarted = deferred<void>();
  output.slack.filesUploadV2 = async () => {
    uploadStarted.resolve();
    return upload.promise;
  };
  const result: Answer = {
    ...answer,
    opportunities: [
      {
        category: "Reliability",
        summary: "The observed tool failed",
        impact: 8,
        frequency_metric: "errors",
        sources: [],
        frequency: {
          count: 1,
          total: 2,
          label: "1/2 observed calls",
          title: "Tool errors",
          unit: "calls",
        },
      },
    ],
  };
  const chat = new Chat(
    event,
    async () => result,
    output.reply,
    () => now,
    undefined,
    async () => {},
    {
      acknowledge: output.acknowledge,
      timeoutMs: 1000,
      deliveryTimeoutMs: 150,
      cleanupTimeoutMs: 30,
    },
  );
  const running = chat.mention(event);
  await within(uploadStarted.promise, 500);
  await within(running, 500);
  assert.equal(output.posted.length, 1);
  assert.equal(output.updated.length, 2);
  assert(
    JSON.stringify(output.updated.at(-1)).includes("Evidence unavailable"),
  );
  upload.resolve({
    ok: true,
    files: [{ ok: true, files: [{ id: "F123ABC" }] }],
  });
  await tick();
  assert.equal(output.updated.length, 2);
});

test("hung context and claim reads finish visibly for mentions while unknown unmentioned threads stay quiet", async () => {
  for (const stage of ["context", "claim"] as const) {
    const output = transport();
    let models = 0;
    const registry: ThreadRegistry = {
      read: async () =>
        stage === "context"
          ? new Promise(() => {})
          : {
              title: "Finding",
              issue: "1",
              traces: [],
              turns: [],
              handled: [],
            },
      claim: async () => new Promise(() => {}),
      register: async () => {},
      remember: async () => {},
    };
    const chat = new Chat(
      event,
      async () => {
        models++;
        return answer;
      },
      output.reply,
      () => now,
      registry,
      async () => {},
      {
        acknowledge: output.acknowledge,
        contextTimeoutMs: 10,
        deliveryTimeoutMs: 100,
      },
    );
    if (stage === "context") {
      await within(
        chat.mention({
          ...event,
          thread: "1799999900.000000",
          addressed: false,
        }),
        500,
      );
      assert.equal(output.posted.length, 0);
    }
    await within(
      chat.mention({
        ...event,
        ts: "1800000001.000000",
        thread: "1799999900.000000",
      }),
      500,
    );
    assert.equal(models, 0);
    assert.equal(output.posted.length, 1);
    assert(JSON.stringify(output.posted).includes("Evidence unavailable"));
  }
});
