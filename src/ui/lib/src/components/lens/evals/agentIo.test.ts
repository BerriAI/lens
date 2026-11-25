import { describe, expect, it } from "vitest";
import { buildAgentIo, EMPTY_AGENT_IO, type AgentIoDraft } from "./agentIo";
import { httpAgentIo } from "./runs/testRuns";

describe("HTTP agent mapping", () => {
  it("maps dataset input and a stable request ID into an immediate request", () => {
    expect(buildAgentIo(EMPTY_AGENT_IO)).toEqual({
      ok: true,
      value: httpAgentIo,
    });
  });

  it("saves polling, output, follow-ups and trace mappings", () => {
    const result = buildAgentIo({
      ...EMPTY_AGENT_IO,
      submitPath: "/tasks",
      acceptedStatus: "202",
      body: '{"messages":[{"content":""}],"followups":[]}',
      inputPointer: "/messages/0/content",
      requestIdPointer: "",
      followupsPointer: "/followups",
      completion: "poll",
      pollPath: "/tasks/{id}",
      errorPointer: "/error",
      success: " done, idle ",
      requiredArray: "/messages",
      requiredMatches: '{"role":"assistant","complete":true}',
      outputPointer: "/answer",
      trace: true,
      traceSource: "completed",
      traceAttribute: "trace_id",
    });
    expect(result).toEqual({
      ok: true,
      value: {
        ...httpAgentIo,
        submit: {
          method: "POST",
          path: "/tasks",
          accepted_status: 202,
          json: { messages: [{ content: "" }], followups: [] },
        },
        input: [
          { source: "case.input", target: "/messages/0/content" },
          { source: "case.followups", target: "/followups" },
        ],
        completion: {
          kind: "poll",
          id_pointer: "/id",
          path_template: "/tasks/{id}",
          status_pointer: "/status",
          success: ["done", "idle"],
          failure: ["failed", "cancelled"],
          error_pointer: "/error",
          interval_ms: 1000,
          timeout_ms: 600000,
          require_item: {
            array_pointer: "/messages",
            matches: { role: "assistant", complete: true },
          },
        },
        output: { pointer: "/answer", require_nonempty: true },
        trace: { source: "completed", attribute: "trace_id", pointer: "/id" },
      },
    });
  });

  it.each<[string, Partial<AgentIoDraft>]>([
    ["malformed JSON", { body: "{" }],
    ["a non-object body", { body: "[]" }],
    ["a missing target", { inputPointer: "/missing" }],
    ["an object target", { body: '{"input":{},"request_id":""}' }],
    [
      "a populated array target",
      { body: '{"input":["fixed"],"request_id":""}' },
    ],
    ["overlapping inputs", { requestIdPointer: "/input" }],
    ["an invalid pointer escape", { outputPointer: "/bad~2field" }],
    ["JSONPath", { outputPointer: "$.output" }],
    ["a network-path reference", { submitPath: "//external.test/invoke" }],
    ["a full URL", { submitPath: "https://external.test/invoke" }],
    ["a path with query", { submitPath: "/invoke?token=x" }],
    ["a failed status", { acceptedStatus: "400" }],
    ["a bad profile name", { connection: "agent.service" }],
    ["no polling placeholder", { completion: "poll", pollPath: "/runs" }],
    [
      "two polling placeholders",
      { completion: "poll", pollPath: "/runs/{id}/{id}" },
    ],
    ["overlapping states", { completion: "poll", success: "failed" }],
    ["an empty state", { completion: "poll", failure: "failed," }],
    [
      "an interval beyond the deadline",
      { completion: "poll", intervalMs: "2000", timeoutMs: "1000" },
    ],
    [
      "a poll interval below the minimum",
      { completion: "poll", intervalMs: "100" },
    ],
    ["an empty required item", { completion: "poll", requiredArray: "/items" }],
    [
      "non-scalar match fields",
      {
        completion: "poll",
        requiredArray: "/items",
        requiredMatches: '{"value":{}}',
      },
    ],
  ])("rejects %s before saving", (_, changes) => {
    expect(buildAgentIo({ ...EMPTY_AGENT_IO, ...changes }).ok).toBe(false);
  });

  it("supports escaped field names and the whole output string", () => {
    expect(
      buildAgentIo({
        ...EMPTY_AGENT_IO,
        body: '{"a/b~c":""}',
        inputPointer: "/a~1b~0c",
        requestIdPointer: "",
        outputPointer: "",
        requireNonempty: false,
      }),
    ).toMatchObject({
      ok: true,
      value: { output: { pointer: "", require_nonempty: false } },
    });
  });
});
