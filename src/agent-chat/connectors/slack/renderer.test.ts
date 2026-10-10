import assert from "node:assert/strict";
import { test } from "node:test";
import { postCandidate } from "./findings.js";
import type { PersistedCandidate } from "@litellm/lens-agent/findings";
import type { Sample } from "@litellm/lens-agent/evidence";
import type { Config } from "./config.js";

test("a proposed issue has its stable number, status border and private chart in the first message, with receipts in the thread", async () => {
  const messages: Record<string, unknown>[] = [];
  const updates: Record<string, unknown>[] = [];
  const uploads: Record<string, unknown>[] = [];
  const candidate: PersistedCandidate = {
    issueNumber: 7,
    native: {
      lensId: "lens1",
      findingId: "agent-" + "a".repeat(64),
      url:
        "https://lens.example.com/?tab=findings&issue=lens1:agent-" +
        "a".repeat(64),
    },
    fingerprint: "fingerprint",
    evidenceHash: "hash",
    evidenceLinks: ["https://lens.example.com/?trace=one"],
    codeLinks: ["https://github.com/example/repo/blob/sha/tool.ts#L1"],
    frequency: {
      count: 4,
      total: 5,
      title: "Repository calls fail",
      label: "4/5 completed repository calls failed",
      unit: "completed repository calls",
    },
    receipts: [
      {
        user: "person@example.com",
        request: "Please open my repository",
        behavior: "Could not list repositories",
        quote: "HTTP 503 unavailable",
        url: "https://lens.example.com/?trace=one",
      },
    ],
    candidate: {
      kind: "issue",
      category: "Reliability",
      confidence: 0.9,
      impact: 8,
      frequency_metric: "tool_repo",
      issue_key: "repository-list",
      title: "Repository calls fail",
      observation: "Listing returns service errors",
      code_hypothesis: "Upstream routing could explain the errors",
      experiment: "Freeze requests and compare baseline and candidate outcomes",
      outcome: "users could open their repository without retrying",
      limitation: "No benchmark yet",
      evidence: [
        { trace_id: "one", span_id: "tool", quote: "HTTP 503 unavailable" },
      ],
      code: [{ path: "tool.ts", quote: "callRepositories()" }],
    },
  };
  const slack = {
    filesUploadV2: async (options: Record<string, unknown>) => {
      uploads.push(options);
      return {
        ok: true,
        files: [{ ok: true, files: [{ id: "F0123ABC" }] }],
      };
    },
    chat: {
      postMessage: async (message: Record<string, unknown>) => {
        messages.push(message);
        return { ok: true, ts: "123.4" };
      },
      update: async (message: Record<string, unknown>) => {
        updates.push(message);
        return { ok: true };
      },
    },
  } as unknown as Parameters<typeof postCandidate>[0];
  const sample = { traces: [] } as unknown as Sample;
  const source = {
    model: "test-model",
    prompt_revision: "test",
    trace_ids: ["one"],
    repository_sha: "abc123",
    at: 0,
  };
  await postCandidate(
    slack,
    { channel: "C123" } as Config,
    candidate,
    sample,
    source,
  );
  assert.equal(messages.length, 1);
  assert.equal(updates.length, 1);
  assert.equal(updates[0]?.ts, "123.4");
  const first = JSON.stringify(updates[0]);
  assert(
    first.includes("#8058F4") &&
      first.includes("Finding #aaaaaaaa") &&
      first.includes(candidate.native.url) &&
      first.includes("Proposed"),
  );
  assert(
    first.includes("Bug Fix: Reliability") &&
      first.includes("What this can improve"),
  );
  assert(first.includes('"slack_file":{"id":"F0123ABC"}'));
  assert(!first.includes("User tried"));
  assert.equal(uploads[0]?.thread_ts, "123.4");
  assert.equal(uploads[0]?.channel_id, "C123");
  assert(JSON.stringify(uploads[0]?.blocks).includes("User tried"));
  messages.length = 0;
  updates.length = 0;
  slack.filesUploadV2 = async () => {
    throw new Error("private upload failed");
  };
  await assert.rejects(
    postCandidate(
      slack,
      { channel: "C123" } as Config,
      candidate,
      sample,
      source,
    ),
  );
  assert.equal(messages.length, 1);
  assert.equal(updates.length, 0);
});
