import type { EvalDefinition, Scorer } from "./types";

export interface ConnectTarget {
  readonly definition: EvalDefinition;
  readonly dataset: string;
  readonly revision: number;
  readonly baseUrl: string;
}

const pyString = (value: string) => JSON.stringify(value);

function scorerCall(scorer: Scorer): string {
  switch (scorer.kind) {
    case "task_completed":
      return "scorers.task_completed()";
    case "called_before":
      return `scorers.called_before(${pyString(scorer.first)}, ${pyString(scorer.then)})`;
    case "judge":
      return `judge(${pyString(scorer.prompt)})`;
  }
}

function gateCall(definition: EvalDefinition): string {
  const gate = definition.spec.gate;
  const fields = [
    ["regressions", gate.regressions],
    ["critical", gate.critical],
    ["pass_rate", gate.pass_rate],
    ["cost_per_case", gate.cost_per_case],
  ].filter(([, value]) => value !== null && value !== undefined);
  return `Gate(${fields.map(([key, value]) => `${key}=${value}`).join(", ")})`;
}

const ACTION =
  "BerriAI/lens/src/sdk/action@ba921dfbf3cd10a71b43bf1d756ef56ffc0fe69b";

export const evalModule = (agent: string) =>
  agent
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "") || "agent";

export function workflowSnippet(target: ConnectTarget): string {
  return `name: Lens evals
on:
  push:
    branches: [main]
  pull_request:
permissions:
  contents: read
  checks: write
  pull-requests: write
jobs:
  lens:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
      - uses: actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065
        with:
          python-version: "3.12"
      - run: pip install -e .
      - uses: ${ACTION}
        with:
          sdk-token: \${{ secrets.LENS_SDK_TOKEN }}
          api-key: \${{ secrets.LENS_API_KEY }}
          base-url: ${target.baseUrl}
`;
}

export function pyprojectSnippet(target: ConnectTarget): string {
  return `[dependency-groups]
dev = ["lens-evals"]

[tool.lens]
project = ${JSON.stringify(target.definition.spec.agent)}
evals = "evals/"
base_url = ${JSON.stringify(target.baseUrl)}
`;
}

export function evalSnippet(target: ConnectTarget): string {
  const { definition } = target;
  const uses = definition.spec.scorers.some((scorer) => scorer.kind === "judge");
  return `from lens import Eval, Gate, Run, ${uses ? "judge, " : ""}scorers


async def task(case):
    session_id = await start_agent_run(case.input)  # your agent's API
    return Run(trace={"session.id": session_id})


evaluation = Eval(
    ${pyString(definition.name)},
    task=task,
    data="${target.dataset}@${target.revision}",
    scores=[${definition.spec.scorers.map(scorerCall).join(", ")}],
    baseline=${pyString(definition.spec.baseline)},
    trials=${definition.spec.trials},
    gate=${gateCall(definition)},
)
`;
}

export function githubAppInstallUrl(slug: string | undefined): string | null {
  const trimmed = slug?.trim();
  return trimmed
    ? `https://github.com/apps/${encodeURIComponent(trimmed)}/installations/new`
    : null;
}
