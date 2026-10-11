import assert from "node:assert/strict";
import { test } from "node:test";
import { Evidence, type Sample } from "./evidence.js";
import { verifyDocumentation } from "./documentation.js";

const question = "How do team and project budgets interact?";
const source = "A project belongs to a team and has its own budget.";
const sample: Sample = {
  agent: "docs-search",
  window: "test sample",
  scanned_rows: 1,
  matched_rows: 1,
  incomplete: false,
  sampled_for_detail: false,
  population: {
    inspected_traces: 1,
    root_error_traces: 0,
    span_error_traces: 0,
    terminal_traces: 1,
    p95_duration_ms: null,
    above_p95: null,
  },
  warning: "bounded",
  traces: [
    {
      trace_id: "conversation",
      trace_ref: "revision",
      agent_names: ["docs-search"],
      service: "docs-search",
      start_time: "2026-01-01",
      duration_ms: 100,
      status: "ok",
      llm_calls: 1,
      tool_calls: 0,
      error_count: 0,
      input_tokens: 1,
      output_tokens: 1,
      url: "https://lens.example.com/?trace=conversation",
    },
  ],
};
const client = {
  get: async (path: string) =>
    path.includes("/spans/")
      ? {
          span_id: path.split("/spans/")[1]!.split("?")[0],
          input: question,
          output: "",
          attributes: { "gen_ai.operation.name": "invoke_agent" },
        }
      : {
          summary: {
            trace_id: "conversation",
            agent_names: ["docs-search"],
            service: "docs-search",
          },
          spans: [
            {
              span_id: "root",
              parent_span_id: null,
              type: "agent",
              name: "docs-search",
              status: "ok",
              duration_ms: 100,
              error: null,
            },
            {
              span_id: "child",
              parent_span_id: "root",
              type: "agent",
              name: "docs-search",
              status: "ok",
              duration_ms: 10,
              error: null,
            },
          ],
        },
  link: (query: Record<string, string>) =>
    `https://lens.example.com/?${new URLSearchParams(query)}`,
};

test("text input is opt-in and only establishes a request on the selected agent root", async () => {
  const signal = AbortSignal.timeout(5000);
  const standard = new Evidence(sample, client, "docs-search");
  await standard.loadRoots(signal);
  assert.equal(standard.report().records[0]?.readable_request, false);
  const evidence = new Evidence(sample, client, "docs-search", "text");
  await evidence.loadRoots(signal);
  const root = evidence.report().records[0]!;
  assert.equal(root.input, question);
  assert.equal(root.classification, "interactive_root");
  assert(root.incident);
  await evidence.span("conversation", "child", signal);
  assert.deepEqual(
    evidence.observations.get("conversation:child")?.humanInput,
    [],
  );
  assert.equal(evidence.observations.get("conversation:child")?.incident, null);
});

test("documentation findings require a reader request and a verified docs source, excluding agent implementation and performance", async () => {
  const evidence = new Evidence(sample, client, "docs-search", "text");
  await evidence.loadRoots(AbortSignal.timeout(5000));
  const candidate = {
    feedback_type: "documentation_gap",
    product_support: "unverified",
    documentation_gap: "unclear_explanation",
    kind: "opportunity",
    category: "Agent quality",
    confidence: 0.9,
    impact: 5,
    frequency_metric: "unknown",
    issue_key: "budget-precedence",
    title: "Budget guidance needs a team and project precedence example",
    observation: "A reader asks how two documented limits interact",
    code_hypothesis:
      "The inspected project guide lists budgets without explaining precedence",
    experiment:
      "Add an example showing which request is rejected when either budget is exhausted",
    outcome: "readers can configure both limits with a clear expectation",
    limitation: "One observed request; only the cited guide was inspected",
    evidence: [{ trace_id: "conversation", span_id: "root", quote: question }],
    code: [{ path: "docs/projects.mdx", quote: source }],
  };
  const repo = {
    files: new Map([
      [
        "docs/projects.mdx",
        {
          path: "docs/projects.mdx",
          content: source,
          url: "https://github.com/example/docs/blob/revision/docs/projects.mdx",
        },
      ],
      [
        "search/answer.js",
        {
          path: "search/answer.js",
          content: source,
          url: "https://github.com/example/docs/blob/revision/search/answer.js",
        },
      ],
    ]),
  };
  const verified = verifyDocumentation(candidate, evidence, repo);
  assert(verified);
  assert.equal(verified.receipts?.[0]?.request, question);
  assert(verified.codeLinks[0]?.endsWith("docs/projects.mdx#L1"));
  for (const type of ["user_need", "capability_request"] as const) {
    const learning = verifyDocumentation(
      { ...candidate, feedback_type: type, documentation_gap: "none" },
      evidence,
      repo,
    );
    assert(learning);
    assert.deepEqual(learning.feedback, {
      type,
      productSupport: "unverified",
    });
  }
  for (const rejected of [
    { ...candidate, feedback_type: "none" },
    { ...candidate, feedback_type: "agent_health" },
    { ...candidate, product_support: "unsupported_because_not_found" },
    { ...candidate, feedback_type: "user_need", kind: "issue" },
    { ...candidate, frequency_metric: "root_errors" },
    { ...candidate, category: "Performance" },
    { ...candidate, documentation_gap: "none" },
    { ...candidate, documentation_gap: "retrieval_failure" },
    { ...candidate, kind: "regression" },
    { ...candidate, code: [{ path: "search/answer.js", quote: source }] },
    { ...candidate, code: [{ path: "docs/unread.mdx", quote: source }] },
    {
      ...candidate,
      code: [
        {
          path: "docs/projects.mdx",
          quote: "An invented claim about budget precedence",
        },
      ],
    },
    {
      ...candidate,
      evidence: [
        {
          trace_id: "conversation",
          span_id: "root",
          quote: "A fabricated reader request",
        },
      ],
    },
  ])
    assert.equal(verifyDocumentation(rejected, evidence, repo), undefined);
});
