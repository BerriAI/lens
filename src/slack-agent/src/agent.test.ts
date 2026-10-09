import assert from "node:assert/strict";
import { test } from "node:test";
import { Runner } from "@openai/agents";
import {
  ScriptedModel,
  assistantMessage,
  functionCall,
} from "@openai/agents/testing";
import { responder } from "./agent.js";
import type { Config } from "./config.js";

const config = { agent: "selected", model: "test-model" } as Config;

test("official SDK executes the read tool, carries its evidence to the model, and bounds repeated tool reads", async () => {
  const model = new ScriptedModel([
    [functionCall("findings", {}, { callId: "first" })],
    [functionCall("findings", {}, { callId: "repeat" })],
    [
      assistantMessage(
        JSON.stringify({
          title: "Candidate",
          summary: "One candidate; no measured gain is established",
          opportunities: [],
          sources: [],
        }),
      ),
    ],
  ]);
  let reads = 0;
  const answer = await responder(
    config,
    {
      read: async () => {
        reads++;
        return JSON.stringify({ candidate: "fixture evidence" });
      },
    },
    new Runner({
      modelProvider: { getModel: async () => model },
      tracingDisabled: true,
    }),
  )(
    [{ role: "user", content: "What should improve?" }],
    AbortSignal.timeout(10_000),
  );
  assert.equal(reads, 1);
  assert.equal(model.calls.length, 3);
  assert(
    JSON.stringify(model.calls[1]?.request.input).includes("fixture evidence"),
  );
  assert(answer.summary.includes("no measured gain"));
  assert.equal(model.firstCall?.request.modelSettings.store, false);
  model.assertComplete();
});

test("SDK turn limit stops a model that continues asking for evidence", async () => {
  const model = new ScriptedModel(
    Array.from({ length: 12 }, (_, i) => [
      functionCall("recent_traces", {}, { callId: String(i) }),
    ]),
  );
  const answer = responder(
    config,
    { read: async () => "bounded evidence" },
    new Runner({
      modelProvider: { getModel: async () => model },
      tracingDisabled: true,
    }),
  );
  await assert.rejects(
    answer(
      [{ role: "user", content: "continue forever" }],
      AbortSignal.timeout(10_000),
    ),
    /Max turns/i,
  );
  assert.equal(model.calls.length, 8);
});
