import { describe, expect, it } from "vitest";
import {
  evalFilePath,
  evalModule,
  evalSnippet,
  githubRunRepository,
  parseGitHubRepository,
  parseTaskImport,
  pyprojectSnippet,
  savedEvalTestSnippet,
  workflowSnippet,
  type ConnectTarget,
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
        {
          kind: "called_before" as const,
          first: "lookup_order",
          then: "issue_refund",
        },
      ],
      trials: 3,
      baseline: "main",
      gate: {
        regressions: 0,
        critical: null,
        pass_rate: 0.9,
        cost_per_case: null,
        min: {},
      },
      timeout_per_trial_ms: 1_200_000,
    },
    updated_at: "2026-10-08T00:00:00.000Z",
  },
  dataset: "refunds",
  revision: 7,
  baseUrl: "https://lens.test",
} satisfies ConnectTarget;

describe("connect snippets", () => {
  it("publishes through the connected App without requesting workflow write permissions", () => {
    const workflow = workflowSnippet({ ...target, reportViaApp: true });
    expect(workflow).toContain("report-via-app: true");
    expect(workflow).toContain("permissions:\n  contents: read\njobs:");
    expect(workflow).not.toContain("checks: write");
    expect(workflow).not.toContain("pull-requests: write");
  });
  it("points the workflow at this Lens and keeps credentials in repo secrets", () => {
    const workflow = workflowSnippet(target);
    expect(workflow).toContain("base-url: ${{ vars.LENS_BASE_URL }}");
    expect(workflow).toContain("api-key: ${{ secrets.LENS_API_KEY }}");
    expect(workflow).toContain("checks: write");
    expect(workflow).toContain("install-from-source: true");
  });

  it("names the project and pins the dataset revision the eval replays", () => {
    expect(pyprojectSnippet(target)).toBe(`[tool.lens]
project = "Refund Agent"
evals = "evals/refund_gate.py"
base_url = "https://lens.test"
`);
    expect(evalSnippet(target)).toContain('data="refunds@7"');
    expect(workflowSnippet(target)).toContain(
      `path: "${evalFilePath(target)}"`,
    );
  });

  it("writes the stored eval's scorers, trials and gate into the eval file", () => {
    const snippet = evalSnippet(target);
    expect(snippet).toContain('Eval(\n    "refund-gate"');
    expect(snippet).toContain(
      'scores=[scorers.task_completed(), scorers.called_before("lookup_order", "issue_refund")]',
    );
    expect(snippet).toContain("trials=3");
    expect(snippet).toContain("gate=Gate(regressions=0, pass_rate=0.9)");
    expect(snippet).toContain(
      "timeout_per_trial=timedelta(milliseconds=1200000)",
    );
  });

  it("preserves custom judge models, score thresholds, and millisecond timeouts", () => {
    const snippet = evalSnippet({
      ...target,
      definition: {
        ...target.definition,
        spec: {
          ...target.definition.spec,
          scorers: [
            {
              kind: "judge",
              prompt: 'Verify "success".\nExplain why.',
              model: "custom-judge",
            },
          ],
          gate: { min: { 'judge "success"': 0.85 }, cost_per_case: 0.2 },
          timeout_per_trial_ms: 12_345,
        },
      },
    });
    expect(snippet).toContain(
      'judge("Verify \\"success\\".\\nExplain why.", model="custom-judge")',
    );
    expect(snippet).toContain(
      'gate=Gate(cost_per_case=0.2, min={"judge \\"success\\"": 0.85})',
    );
    expect(snippet).toContain(
      "timeout_per_trial=timedelta(milliseconds=12345)",
    );
  });

  it("escapes stored names, dataset names, tool inputs, and server URLs", () => {
    const changed = {
      ...target,
      dataset: 'refunds"\\archive\nnext',
      baseUrl: 'https://lens.test/"quoted"',
      definition: {
        ...target.definition,
        name: 'gate"\nnext',
        spec: {
          ...target.definition.spec,
          scorers: [
            {
              kind: "called_before" as const,
              first: 'lookup"\nnext',
              then: "issue\\refund",
            },
          ],
        },
      },
    };
    const snippet = evalSnippet(changed);
    expect(snippet).toContain('"gate\\"\\nnext"');
    expect(snippet).toContain('data="refunds\\"\\\\archive\\nnext@7"');
    expect(snippet).toContain(
      'scorers.called_before("lookup\\"\\nnext", "issue\\\\refund")',
    );
    expect(pyprojectSnippet(changed)).toContain(
      'base_url = "https://lens.test/\\"quoted\\""',
    );
  });

  it("requires an explicit task implementation when no import has been configured", () => {
    const snippet = evalSnippet(target);
    expect(snippet).toContain("async def task(case: Case) -> Run:");
    expect(snippet).toContain("raise NotImplementedError(");
    expect(snippet).toContain(
      "returns lens.Run with its trace_id or session.id.",
    );
  });

  it("calls the configured async agent task with the whole case", () => {
    const parsed = parseTaskImport("agents.refunds:run_case");
    expect(parsed.ok).toBe(true);
    if (!parsed.ok) return;
    const snippet = evalSnippet({ ...target, taskImport: parsed.value });
    expect(snippet).toContain(
      "from agents.refunds import run_case as agent_task",
    );
    expect(snippet).toContain(
      "async def task(case: Case) -> Run:\n    return await agent_task(case)",
    );
  });

  it("runs only same-repository PRs and allows a manual first run", () => {
    const workflow = workflowSnippet(target);
    expect(workflow).toContain("  workflow_dispatch:\n");
    expect(workflow).toContain(
      "if: ${{ github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository }}",
    );
    expect(workflow).toContain("persist-credentials: false");
    expect(workflow).toContain(
      "group: lens-evals-${{ github.workflow }}-${{ github.event.pull_request.number || github.ref }}",
    );
    expect(workflow).toContain("cancel-in-progress: true");
  });

  it("uses the stored baseline for both the workflow and evaluation", () => {
    const changed = {
      ...target,
      definition: {
        ...target.definition,
        spec: { ...target.definition.spec, baseline: "release" },
      },
    };
    expect(workflowSnippet(changed)).toContain('branches: ["release"]');
    expect(evalSnippet(changed)).toContain('baseline="release"');
  });

  it("quotes branch text and neutralizes workflow expressions in user input", () => {
    const workflow = workflowSnippet({
      ...target,
      definition: {
        ...target.definition,
        spec: {
          ...target.definition.spec,
          baseline: 'release"\\next${{secrets.LENS_API_KEY}}',
        },
      },
    });
    expect(workflow).toContain(
      'branches: ["release\\"\\\\next${{ \'${{\' }}secrets.LENS_API_KEY}}"]',
    );
  });

  it("includes dependency installation only when configured and preserves multiline commands", () => {
    expect(workflowSnippet(target)).not.toContain("Install agent dependencies");
    expect(workflowSnippet({ ...target, installCommand: "  " })).not.toContain(
      "Install agent dependencies",
    );
    const workflow = workflowSnippet({
      ...target,
      installCommand:
        "python -m pip install -r requirements.txt\npython -m pip install -e .",
    });
    expect(workflow).toContain(
      "run: |\n          python -m pip install -r requirements.txt\n          python -m pip install -e .\n      - uses:",
    );
  });

  it.each([
    ["Refund Agent", "refund_agent"],
    ["--", "agent"],
    ["12 refunds", "eval_12_refunds"],
    ["class", "eval_class"],
    ["../../${{ secrets.KEY }}", "secrets_key"],
  ])("derives a python module name from %s", (agent, module) => {
    expect(evalModule(agent)).toBe(module);
  });
});

describe("GitHub repository input", () => {
  it.each([
    "BerriAI/lens",
    " https://github.com/BerriAI/lens ",
    "https://github.com/BerriAI/lens.git/",
  ])("normalizes %s", (input) => {
    expect(parseGitHubRepository(input)).toEqual({
      ok: true,
      value: {
        owner: "BerriAI",
        name: "lens",
        fullName: "BerriAI/lens",
        url: "https://github.com/BerriAI/lens",
      },
    });
  });

  it.each([
    "",
    "lens",
    "owner/../repo",
    "owner/..",
    "https://github.com/owner/repo/tree/main",
    "https://github.com/owner/repo?token=value",
    "https://github.com.evil.test/owner/repo",
    "https://github.com@evil.test/owner/repo",
    "http://github.com/owner/repo",
    "owner/${{secrets.KEY}}",
  ])("rejects %s without treating it as a linked repository", (input) => {
    expect(parseGitHubRepository(input)).toEqual({
      ok: false,
      error: expect.stringContaining("GitHub repository"),
    });
  });
});

describe("task import input", () => {
  it("returns a typed module and function for the generated import", () => {
    expect(parseTaskImport("  agents.refunds:run_case  ")).toEqual({
      ok: true,
      value: {
        module: "agents.refunds",
        functionName: "run_case",
        reference: "agents.refunds:run_case",
      },
    });
  });

  it.each([
    "",
    "agents/refunds.py:task",
    "agents.refunds.task",
    "agents:task()",
    "agents:async",
    "agents.class:task",
    "agents:task\nprint('oops')",
  ])("rejects invalid Python import %s", (input) => {
    expect(parseTaskImport(input)).toEqual({
      ok: false,
      error: expect.stringContaining("package.module:function"),
    });
  });
});

describe("GitHub run provenance", () => {
  it.each([
    "https://github.com/BerriAI/lens/actions/runs/123",
    "https://github.com/BerriAI/lens/actions/runs/123/attempts/2",
  ])("extracts the repository from a GitHub run URL", (ciUrl) => {
    expect(githubRunRepository(ciUrl)?.fullName).toBe("BerriAI/lens");
  });

  it.each([
    undefined,
    null,
    "",
    "BerriAI/lens",
    "https://github.com/BerriAI/lens",
    "https://github.com.evil.test/BerriAI/lens/actions/runs/123",
    "https://github.com/BerriAI/lens/actions/runs/not-a-run",
    "https://github.com/BerriAI/lens/pull/123",
    "http://github.com/BerriAI/lens/actions/runs/123",
    "https://github.com/BerriAI/lens/actions/runs/123?redirect=evil",
  ])("does not infer an observed GitHub repository from %s", (ciUrl) => {
    expect(githubRunRepository(ciUrl)).toBeNull();
  });
});

describe("saved eval test snippets", () => {
  it("runs the exact saved name through the Python SDK and records real outputs", () => {
    const code = savedEvalTestSnippet({
      ...target.definition,
      name: 'eval "quoted"\nname',
    });
    expect(code).toContain(
      String.raw`with lens.evals.test("eval \"quoted\"\nname") as evaluation:`,
    );
    expect(code).toContain("actual = agent.run(input=case.input)");
    expect(code).toContain(
      "evaluation.record(case, output=actual.output, trace_id=actual.trace_id)",
    );
    expect(code).toContain("evaluation.finish().assert_passed()");
  });

  it("uses the saved HTTP mapping when an agent contract exists", () => {
    const code = savedEvalTestSnippet({
      ...target.definition,
      spec: {
        ...target.definition.spec,
        agent_io: { version: 1, connection: "agent-service" },
      },
    });
    expect(code).toContain('lens.evals.run("refund-gate").assert_passed()');
    expect(code).not.toContain("from my_agent import agent");
  });
});
