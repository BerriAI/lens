import assert from "node:assert/strict";
import { test } from "node:test";
import { Chat, type Mention } from "./chat.js";
import {
  bootstrapThreads,
  candidateThread,
  FindingThreads,
  type FindingThread,
} from "./threads.js";
import type { Config } from "./config.js";
import type { Snapshot, StateStore } from "@litellm/lens-agent/state";
import type { Turn } from "@litellm/lens-agent/models";
import type { Sample } from "@litellm/lens-agent/evidence";

const config = {
  workspace: "TTEST",
  channel: "CTEST",
  agent: "example-agent",
  publicUrl: "https://lens.example.com",
} as Config;
const now = 1_800_000_000_000;
const root = "1799999900.000000";
const finding: FindingThread = {
  issue: "12",
  title: "Reliability: Repository calls fail",
  traces: [
    {
      trace_id: "trace1",
      trace_ref: "ref1",
      service: "example-agent",
      agent_names: ["example-agent"],
      start_time: "2026-01-01",
      duration_ms: 180000,
      status: "error",
      llm_calls: 1,
      tool_calls: 1,
      error_count: 1,
      input_tokens: 1,
      output_tokens: 1,
      url: "https://lens.example.com/?trace=trace1&trace_ref=ref1",
    },
  ],
  turns: [],
  handled: [],
};

function stores() {
  const values = new Map<string, Snapshot<FindingThread | null>>();
  return (key: string): StateStore<FindingThread | null> => ({
    read: async () =>
      values.get(key) ?? { key, revision: 0, digest: "", value: null },
    commit: async (previous, value) => {
      assert.equal(previous.revision, values.get(key)?.revision ?? 0);
      const next = { ...previous, revision: previous.revision + 1, value };
      values.set(key, next);
      return next;
    },
  });
}

const event = (overrides: Partial<Mention> = {}): Mention => ({
  id: "message",
  workspace: config.workspace,
  channel: config.channel,
  user: "UUSER",
  text: "Why did this tool fail?",
  thread: root,
  ts: "1800000000.000000",
  addressed: false,
  ...overrides,
});

test("only finding threads accept unmentioned replies; persisted context and message claims survive restart", async () => {
  const store = stores();
  const registry = new FindingThreads(store);
  await registry.register(root, finding);
  const requests: { history: readonly Turn[]; finding?: FindingThread }[] = [];
  const states: string[] = [];
  const create = () =>
    new Chat(
      config,
      async (history, _signal, context) => {
        requests.push({ history, finding: context });
        return {
          title: "Reliability",
          summary: "The observed tool returned HTTP 503",
          sources: [],
          opportunities: [],
        };
      },
      async () => {},
      () => now,
      new FindingThreads(store),
      async (_event, state) => {
        states.push(state);
      },
    );
  const chat = create();
  await chat.mention(event({ thread: "1799999901.000000" }));
  await chat.mention(event({ ts: "1800000001.000000", thread: undefined }));
  await chat.mention(event({ ts: "1800000002.000000", channel: "COTHER" }));
  assert.equal(requests.length, 0);
  await chat.mention(event({ ts: "1800000003.000000" }));
  await chat.mention(
    event({ id: "app-mention", addressed: true, ts: "1800000003.000000" }),
  );
  assert.equal(requests.length, 1);
  assert.equal(requests[0]?.finding?.traces[0]?.trace_ref, "ref1");
  const restarted = create();
  await restarted.mention(event({ id: "retry", ts: "1800000003.000000" }));
  await restarted.mention(
    event({
      id: "next",
      ts: "1800000004.000000",
      text: "What should we test? xoxb-private-token",
    }),
  );
  assert.equal(requests.length, 2);
  assert.equal(requests[1]?.history.length, 3);
  assert(requests[1]?.history[1]?.content.includes("HTTP 503"));
  assert(!JSON.stringify(requests[1]?.history).includes("xoxb-private-token"));
  assert(
    !JSON.stringify(await registry.read(root)).includes("xoxb-private-token"),
  );
  assert.deepEqual(states, ["working", "done", "working", "done"]);
});

test("reaction failures do not suppress answers, model failures mark the accepted message failed", async () => {
  const registry = new FindingThreads(stores());
  await registry.register(root, finding);
  const states: string[] = [];
  let replies = 0;
  const chat = new Chat(
    config,
    async () => {
      throw new Error("private error");
    },
    async () => {
      replies++;
    },
    () => now,
    registry,
    async (_event, state) => {
      states.push(state);
      throw new Error("no permission");
    },
  );
  await chat.mention(event());
  assert.equal(replies, 1);
  assert.deepEqual(states, ["working", "failed"]);
});

test("operator bootstrap cannot install cross-agent, cross-origin or mismatched trace identities", async () => {
  const registry = new FindingThreads(stores());
  for (const changed of [
    { service: "other", agent_names: ["other"] },
    { url: "https://other.example.com/?trace=trace1&trace_ref=ref1" },
    { trace_ref: "other-ref" },
  ])
    await assert.rejects(
      bootstrapThreads(
        registry,
        config,
        JSON.stringify([
          {
            thread: root,
            finding: {
              ...finding,
              traces: [{ ...finding.traces[0], ...changed }],
            },
          },
        ]),
      ),
    );
  assert.equal(await registry.read(root), null);
  await bootstrapThreads(
    registry,
    config,
    JSON.stringify([{ thread: root, finding }]),
  );
  assert.equal((await registry.read(root))?.issue, "12");
});

test("finding context preserves posted trace numbering and selected spans independently of sample order", () => {
  const trace = finding.traces[0]!;
  const sample = {
    traces: [trace, { ...trace, trace_id: "trace2", trace_ref: "ref2" }],
  } as Sample;
  const evidence = [
    { trace_id: "trace2", span_id: "tool2", quote: "failed" },
    { trace_id: "trace1", span_id: "tool1", quote: "failed" },
    { trace_id: "trace2", span_id: "second-tool", quote: "failed again" },
    { trace_id: "trace1", span_id: "root", quote: "request" },
  ];
  const links = evidence.map(
    (item) =>
      `https://lens.example.com/?trace=${item.trace_id}&span=${item.span_id}`,
  );
  const context = candidateThread(
    {
      issueNumber: 12,
      native: {
        lensId: "lens1",
        findingId: "agent-" + "a".repeat(64),
        url:
          "https://lens.example.com/?tab=findings&issue=lens1:agent-" +
          "a".repeat(64),
      },
      candidate: { title: "Failure", evidence },
      evidenceLinks: links,
    },
    sample,
  );
  assert.deepEqual(
    context.traces.map((item) => [item.trace_id, item.span_id, item.url]),
    evidence.map((item, i) => [item.trace_id, item.span_id, links[i]]),
  );
  assert.equal(context.issue, "agent-" + "a".repeat(64));
});

test("unavailable durable context gives an explicit mention an error and leaves unmentioned threads quiet", async () => {
  const store: StateStore<FindingThread | null> = {
    read: async () => {
      throw new Error("private database error");
    },
    commit: async () => {
      throw new Error("unexpected write");
    },
  };
  let models = 0;
  const replies: string[] = [];
  const chat = new Chat(
    config,
    async () => {
      models++;
      return { title: "Unexpected", summary: "", sources: [] };
    },
    async (_channel, _thread, answer) => {
      replies.push(answer.summary);
    },
    () => now,
    new FindingThreads(() => store),
  );
  await chat.mention(event());
  await chat.mention(event({ addressed: true, ts: "1800000001.000000" }));
  assert.equal(models, 0);
  assert.equal(replies.length, 1);
  assert(replies[0]?.includes("finding context"));
  assert(!replies[0]?.includes("private"));
});
