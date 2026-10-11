import assert from "node:assert/strict";
import { test } from "node:test";
import { Chat, type Mention } from "./chat.js";
import {
  configFrom,
  channelConfigs,
  agentConfig,
  investigatorOptions,
  localApiUrl,
  type Config,
} from "./config.js";
import type { Turn } from "@litellm/lens-agent/models";

export const config: Config = {
  botToken: "bot-test",
  appToken: "app-test",
  workspace: "TTEST",
  channel: "CTEST",
  agent: "example-agent",
  publicUrl: "https://lens.example.com",
  apiUrl: "http://127.0.0.1:4100",
  lensKey: "local-test-key",
  openaiKey: "model-test-key",
  openaiBaseUrl: "https://api.openai.com/v1",
  model: "test-model",
};
const now = 1_800_000_000_000;
const mention = (overrides: Partial<Mention> = {}): Mention => ({
  id: "event-one",
  workspace: config.workspace,
  channel: config.channel,
  user: "UUSER",
  text: "<@UBOT> What broke?",
  ts: `${now / 1000}.000000`,
  ...overrides,
});

test("chat ignores other workspaces, channels, bots, stale events and duplicate deliveries before invoking the model", async () => {
  const calls: (readonly Turn[])[] = [];
  const replies: string[] = [];
  const chat = new Chat(
    config,
    async (history) => {
      calls.push(history);
      return { title: "Lens", summary: "Evidence answer", sources: [] };
    },
    async (_c, _t, text) => {
      replies.push(text.summary);
    },
    () => now,
  );
  for (const event of [
    mention({ workspace: "TOTHER" }),
    mention({ channel: "COTHER" }),
    mention({ bot: true }),
    mention({ ts: "1.000000" }),
    mention({ ts: "NaN" }),
  ])
    await chat.mention(event);
  assert.equal(calls.length, 0);
  await chat.mention(mention());
  await chat.mention(mention());
  assert.equal(calls.length, 1);
  assert.equal(replies.length, 1);
  assert.equal(calls[0]?.[0]?.content, "What broke?");
});

test("follow-up mentions retain only that thread's bounded conversation", async () => {
  const calls: (readonly Turn[])[] = [];
  const replies: [string, string][] = [];
  const chat = new Chat(
    config,
    async (history) => {
      calls.push(history);
      return { title: "Lens", summary: "answer", sources: [] };
    },
    async (channel, thread) => {
      replies.push([channel, thread]);
    },
    () => now,
  );
  await chat.mention(mention());
  await chat.mention(
    mention({ id: "two", ts: "1800000001.000000", thread: mention().ts }),
  );
  await chat.mention(mention({ id: "three", ts: "1800000002.000000" }));
  assert.equal(calls[1]?.length, 3);
  assert.equal(calls[2]?.length, 1);
  assert.deepEqual(replies[1], [config.channel, mention().ts]);
});

test("model failures release concurrency and give no health conclusion", async () => {
  const replies: string[] = [];
  let calls = 0;
  const chat = new Chat(
    config,
    async () => {
      calls++;
      throw new Error("secret-model-response");
    },
    async (_c, _t, text) => {
      replies.push(text.summary);
    },
    () => now,
  );
  await chat.mention(mention());
  await chat.mention(mention({ id: "two", ts: "1800000001.000000" }));
  assert.equal(calls, 2);
  assert(
    replies.every(
      (text) => text.includes("not evidence") && !text.includes("secret"),
    ),
  );
});

test("a concurrent mention and hourly budget cannot start additional model requests", async () => {
  let release: (() => void) | undefined;
  let calls = 0;
  const waiting = new Promise<void>((resolve) => {
    release = resolve;
  });
  const chat = new Chat(
    config,
    async () => {
      calls++;
      await waiting;
      return { title: "Lens", summary: "answer", sources: [] };
    },
    async () => {},
    () => now,
  );
  const first = chat.mention(mention());
  await new Promise<void>((resolve) => setImmediate(resolve));
  await chat.mention(mention({ id: "concurrent", ts: "1800000001.000000" }));
  assert.equal(calls, 1);
  release!();
  await first;
  for (let i = 0; i < 25; i++)
    await chat.mention(
      mention({
        id: `later-${i}`,
        ts: `18000000${String(i + 2).padStart(2, "0")}.000000`,
      }),
    );
  assert.equal(calls, 20);
});

test("disabled chat needs no credentials; enabled chat rejects ambiguous tenant scope", () => {
  assert.equal(configFrom({}), undefined);
  assert.throws(
    () =>
      configFrom({
        LENS_SLACK_CHAT_ENABLED: "true",
        LENS_SLACK_TEAM_ID: "tenant",
      }),
    /ALL_TEAMS/,
  );
  assert.equal(localApiUrl("0.0.0.0:4100"), "http://127.0.0.1:4100");
  assert.equal(localApiUrl("[::]:4101"), "http://[::1]:4101");
  assert.throws(() => localApiUrl("example.com:4318"));
});

test("additional channels keep their own agent, repository and native findings destination", () => {
  const base = {
    workspace: "T123",
    channel: "C123",
    agent: "original",
    service: "original-service",
    findingLensId: "original-lens",
    botToken: "bot",
    appToken: "app",
  } as Config;
  const target = {
    channel: "C456",
    agent: "docs",
    repository: "example/docs",
    findingLensId: "docs-lens",
    focus: "documentation",
    rootInput: "text",
  };
  const env = {
    LENS_SLACK_ADDITIONAL_TARGETS: JSON.stringify([target]),
    LENS_INVESTIGATOR_ENABLED: "true",
    LENS_INVESTIGATOR_REPOSITORY: "example/original",
    LENS_GITHUB_CLIENT_ID: "client",
    LENS_GITHUB_PRIVATE_KEY: "key",
    CLICKHOUSE_URL: "http://localhost:8123",
    LENS_INVESTIGATOR_KNOWN_FINDINGS: "original-seed",
  };
  const configs = channelConfigs(base, env);
  assert.equal(configs[0], base);
  const docs = configs[1]!;
  assert.equal(docs.service, "docs");
  assert.equal(docs.findingLensId, "docs-lens");
  assert.equal(docs.botToken, base.botToken);
  assert.equal(agentConfig(docs).focus, "documentation");
  assert.equal(agentConfig(docs).rootInput, "text");
  assert.deepEqual(investigatorOptions(docs, env)?.scope, [
    "T123",
    "C456",
    "docs",
    "example/docs",
  ]);
  assert.equal(investigatorOptions(docs, env)?.knownFindings, undefined);
  assert.equal(investigatorOptions(base, env)?.knownFindings, "original-seed");
  for (const invalid of [
    [{ ...target, channel: base.channel }],
    [target, target],
    [{ ...target, findingLensId: "" }],
    [{ ...target, channel: "#docs" }],
    [{ ...target, repository: "../another/repo" }],
    [{ ...target, botToken: "override" }],
  ])
    assert.throws(() =>
      channelConfigs(base, {
        LENS_SLACK_ADDITIONAL_TARGETS: JSON.stringify(invalid),
      }),
    );
});
