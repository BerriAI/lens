import assert from "node:assert/strict";
import { test } from "node:test";
import type { AssistantThreadsSetStatusArguments } from "@slack/web-api";
import { workingStatus } from "./activity.js";
import type { Mention } from "./chat.js";

const event: Mention = {
  id: "event",
  workspace: "TTEST",
  channel: "CTEST",
  user: "UUSER",
  ts: "1800000001.000000",
  thread: "1800000000.000000",
  text: "Why did the tool fail?",
};

test("working status refreshes in the original thread and clears on either terminal state", async (t) => {
  t.mock.timers.enable({ apis: ["setInterval"] });
  for (const state of ["done", "failed"] as const) {
    const calls: AssistantThreadsSetStatusArguments[] = [];
    const activity = workingStatus(async (args) => {
      calls.push(args);
    });
    await activity(event, "working");
    await activity(event, "working");
    t.mock.timers.tick(60_000);
    await activity(event, state);
    t.mock.timers.tick(180_000);
    await Promise.resolve();
    assert.deepEqual(calls, [
      {
        channel_id: event.channel,
        thread_ts: event.thread,
        status: "is working…",
      },
      {
        channel_id: event.channel,
        thread_ts: event.thread,
        status: "is working…",
      },
      { channel_id: event.channel, thread_ts: event.thread, status: "" },
    ]);
  }
});

test("a delayed start cannot restore a completed indicator and another message cannot clear it", async () => {
  const calls: string[] = [];
  let release = () => {};
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  const activity = workingStatus(async ({ status }) => {
    calls.push(status);
    if (status) await pending;
  });
  const started = activity(event, "working");
  await activity({ ...event, ts: "1800000002.000000" }, "failed");
  const stopped = activity(event, "failed");
  assert.deepEqual(calls, ["is working…"]);
  release();
  await Promise.all([started, stopped]);
  assert.deepEqual(calls, ["is working…", ""]);
});

test("Slack status failures leave the reply lifecycle usable and a root mention targets its own thread", async () => {
  const calls: AssistantThreadsSetStatusArguments[] = [];
  const activity = workingStatus(async (args) => {
    calls.push(args);
    throw new Error("provider failure with private details");
  });
  const root = { ...event, thread: undefined };
  await activity(root, "working");
  await activity(root, "done");
  assert.deepEqual(
    calls.map(({ thread_ts, status }) => [thread_ts, status]),
    [
      [event.ts, "is working…"],
      [event.ts, ""],
    ],
  );
});
