import { describe, expect, it } from "vitest";
import {
  evalModule,
  evalSnippet,
  githubAppInstallUrl,
  pyprojectSnippet,
  workflowSnippet,
} from "./connectSnippets";

const target = {
  definition: {
    name: "refund-gate",
    spec: {
      agent: "Refund Agent",
      dataset_id: "dataset-1",
      revision: 7,
      scorers: [
        { kind: "task_completed" as const },
        { kind: "called_before" as const, first: "lookup_order", then: "issue_refund" },
      ],
      trials: 3,
      baseline: "main",
      gate: { regressions: 0, critical: null, pass_rate: 0.9, cost_per_case: null, min: {} },
      timeout_per_trial_ms: 1_200_000,
    },
    updated_at: "2026-10-08T00:00:00.000Z",
  },
  dataset: "refunds",
  revision: 7,
  baseUrl: "https://lens.test",
};

describe("connect snippets", () => {
  it("points the workflow at this Lens and keeps credentials in repo secrets", () => {
    const workflow = workflowSnippet(target);
    expect(workflow).toContain("base-url: https://lens.test");
    expect(workflow).toContain("api-key: ${{ secrets.LENS_API_KEY }}");
    expect(workflow).toContain("checks: write");
  });

  it("names the project and pins the dataset revision the eval replays", () => {
    expect(pyprojectSnippet(target)).toContain('project = "Refund Agent"');
    expect(evalSnippet(target)).toContain('data="refunds@7"');
  });

  it("writes the stored eval's scorers, trials and gate into the eval file", () => {
    const snippet = evalSnippet(target);
    expect(snippet).toContain('Eval(\n    "refund-gate"');
    expect(snippet).toContain('scores=[scorers.task_completed(), scorers.called_before("lookup_order", "issue_refund")]');
    expect(snippet).toContain("trials=3");
    expect(snippet).toContain("gate=Gate(regressions=0, pass_rate=0.9)");
  });

  it.each([
    ["Refund Agent", "refund_agent"],
    ["--", "agent"],
  ])("derives a python module name from %s", (agent, module) => {
    expect(evalModule(agent)).toBe(module);
  });

  it.each([
    [undefined, null],
    ["  ", null],
    ["litellm-lens", "https://github.com/apps/litellm-lens/installations/new"],
  ])("builds the GitHub App install link from %s", (slug, url) => {
    expect(githubAppInstallUrl(slug)).toBe(url);
  });
});
