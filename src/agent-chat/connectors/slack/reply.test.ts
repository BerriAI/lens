import assert from "node:assert/strict";
import { test } from "node:test";
import { agentConfig, type Config } from "./config.js";
import { replyAgent } from "./reply.js";

test("the connector sends a question to the core and applies Slack safety to its grounded response", async () => {
  const config: Config = {
    agent: "selected",
    model: "test-model",
    service: "selected",
    publicUrl: "https://lens.example.com",
    apiUrl: "http://127.0.0.1:4318",
    lensKey: "lens-key",
    openaiKey: "model-key",
    openaiBaseUrl: "https://api.openai.com/v1",
    botToken: "private-slack-bot",
    appToken: "private-slack-app",
    workspace: "TTEST",
    channel: "CTEST",
  };
  assert(!JSON.stringify(agentConfig(config)).includes("private-slack"));
  assert(!("channel" in agentConfig(config)));
  const url = "https://lens.example.com/?trace=verified";
  let calls = 0;
  const answer = await replyAgent(config, async (history, signal) => {
    calls++;
    assert.equal(history[0]?.content, "Why did this tool fail?");
    assert.equal(signal.aborted, false);
    return {
      answer: {
        title: "Reliability",
        summary: "**Recorded failure**\nToken: xoxb-secret-do-not-post",
        sources: [
          { label: "Trace 1", url },
          { label: "Invented", url: "https://unrelated.example.com" },
        ],
        opportunities: [],
      },
      evidenceUrls: [url],
      frequencies: {},
      detailed: false,
    };
  })(
    [{ role: "user", content: "Why did this tool fail?" }],
    AbortSignal.timeout(1000),
  );
  assert.equal(calls, 1);
  assert.deepEqual(answer.sources, [{ label: "Trace 1", url }]);
  assert(!answer.summary.includes("xoxb-secret"));
});
