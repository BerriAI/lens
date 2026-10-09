import type { EvalDefinition, Scorer } from "./types";

export interface GitHubRepository {
  readonly owner: string;
  readonly name: string;
  readonly fullName: string;
  readonly url: string;
}

export interface TaskImport {
  readonly module: string;
  readonly functionName: string;
  readonly reference: string;
}

export type SetupInput<T> =
  | { readonly ok: true; readonly value: T }
  | { readonly ok: false; readonly error: string };

export interface ConnectTarget {
  readonly definition: EvalDefinition;
  readonly dataset: string;
  readonly revision: number;
  readonly baseUrl: string;
  readonly repository?: GitHubRepository;
  readonly taskImport?: TaskImport;
  readonly installCommand?: string;
  readonly reportViaApp?: boolean;
}

const pyString = (value: string) => JSON.stringify(value);
const yamlString = (value: string) =>
  JSON.stringify(value.replaceAll("${{", "${{ '${{' }}"));

export function parseGitHubRepository(
  input: string,
): SetupInput<GitHubRepository> {
  const match = /^(?:https:\/\/github\.com\/)?([^/]+)\/([^/]+)\/?$/i.exec(
    input.trim(),
  );
  const owner = match?.[1] ?? "";
  const name = (match?.[2] ?? "").replace(/\.git$/, "");
  if (
    !/^[a-z0-9](?:[a-z0-9-]{0,37}[a-z0-9])?$/i.test(owner) ||
    !/^[a-z0-9_.-]{1,100}$/i.test(name) ||
    name === "." ||
    name === ".."
  ) {
    return {
      ok: false,
      error: "Enter a GitHub repository URL or owner/repository.",
    };
  }
  const fullName = `${owner}/${name}`;
  return {
    ok: true,
    value: { owner, name, fullName, url: `https://github.com/${fullName}` },
  };
}

const PYTHON_KEYWORDS = new Set([
  "False",
  "None",
  "True",
  "and",
  "as",
  "assert",
  "async",
  "await",
  "break",
  "class",
  "continue",
  "def",
  "del",
  "elif",
  "else",
  "except",
  "finally",
  "for",
  "from",
  "global",
  "if",
  "import",
  "in",
  "is",
  "lambda",
  "nonlocal",
  "not",
  "or",
  "pass",
  "raise",
  "return",
  "try",
  "while",
  "with",
  "yield",
]);

export function parseTaskImport(input: string): SetupInput<TaskImport> {
  const reference = input.trim();
  const match =
    /^([a-z_][a-z0-9_]*(?:\.[a-z_][a-z0-9_]*)*):([a-z_][a-z0-9_]*)$/i.exec(
      reference,
    );
  if (
    !match ||
    [...match[1].split("."), match[2]].some((name) => PYTHON_KEYWORDS.has(name))
  ) {
    return {
      ok: false,
      error:
        "Use package.module:function for an async task that accepts Case and returns Run.",
    };
  }
  return {
    ok: true,
    value: { module: match[1], functionName: match[2], reference },
  };
}

export function githubRunRepository(
  ciUrl: string | null | undefined,
): GitHubRepository | null {
  const match =
    /^https:\/\/github\.com\/([^/]+\/[^/]+)\/actions\/runs\/[1-9][0-9]*(?:\/attempts\/[1-9][0-9]*)?\/?$/i.exec(
      ciUrl ?? "",
    );
  if (!match) return null;
  const repository = parseGitHubRepository(match[1]);
  return repository.ok ? repository.value : null;
}

function scorerCall(scorer: Scorer): string {
  switch (scorer.kind) {
    case "task_completed":
      return "scorers.task_completed()";
    case "called_before":
      return `scorers.called_before(${pyString(scorer.first)}, ${pyString(scorer.then)})`;
    case "judge":
      return `judge(${pyString(scorer.prompt)}${scorer.model ? `, model=${pyString(scorer.model)}` : ""})`;
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
  const minimums = Object.entries(gate.min ?? {});
  const minimumArgument = minimums.length
    ? [
        `min={${minimums.map(([key, value]) => `${pyString(key)}: ${value}`).join(", ")}}`,
      ]
    : [];
  return `Gate(${[...fields.map(([key, value]) => `${key}=${value}`), ...minimumArgument].join(", ")})`;
}

const ACTION =
  "BerriAI/lens/src/sdk/action@51651cc61bc3863b524683a34f02732e2717b7b7";

export function evalModule(agent: string): string {
  const name =
    agent
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "_")
      .replace(/^_+|_+$/g, "") || "agent";
  return /^[0-9]/.test(name) || PYTHON_KEYWORDS.has(name)
    ? `eval_${name}`
    : name;
}

export const evalFilePath = (target: ConnectTarget): string =>
  `evals/${evalModule(target.definition.name)}.py`;

export function workflowSnippet(target: ConnectTarget): string {
  const install = (target.installCommand?.trim() ?? "")
    .replaceAll("\r\n", "\n")
    .replaceAll("${{", "${{ '${{' }}")
    .split("\n")
    .map((line) => `          ${line}`)
    .join("\n");
  const installStep = target.installCommand?.trim()
    ? `      - name: Install agent dependencies\n        run: |\n${install}\n`
    : "";
  return `name: Lens evals
on:
  push:
    branches: [${yamlString(target.definition.spec.baseline)}]
  pull_request:
    branches: [${yamlString(target.definition.spec.baseline)}]
  workflow_dispatch:
concurrency:
  group: lens-evals-\${{ github.workflow }}-\${{ github.event.pull_request.number || github.ref }}
  cancel-in-progress: true
permissions:
  contents: read
${target.reportViaApp ? "" : "  checks: write\n  pull-requests: write\n"}jobs:
  lens:
    if: \${{ github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository }}
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
        with:
          persist-credentials: false
      - uses: actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065
        with:
          python-version: "3.12"
${installStep}      - uses: ${ACTION}
        with:
          api-key: \${{ secrets.LENS_API_KEY }}
          base-url: \${{ vars.LENS_BASE_URL }}
          path: ${yamlString(evalFilePath(target))}
          install-from-source: true
${target.reportViaApp ? "          report-via-app: true\n" : ""}`;
}

export function pyprojectSnippet(target: ConnectTarget): string {
  return `[tool.lens]
project = ${JSON.stringify(target.definition.spec.agent)}
evals = ${JSON.stringify(evalFilePath(target))}
base_url = ${JSON.stringify(target.baseUrl)}
`;
}

export function evalSnippet(target: ConnectTarget): string {
  const { definition } = target;
  const usesJudge = definition.spec.scorers.some(
    (scorer) => scorer.kind === "judge",
  );
  const usesBuiltIn = definition.spec.scorers.some(
    (scorer) => scorer.kind !== "judge",
  );
  const taskImport = target.taskImport
    ? `\nfrom ${target.taskImport.module} import ${target.taskImport.functionName} as agent_task\n`
    : "";
  const taskBody = target.taskImport
    ? "    return await agent_task(case)"
    : `    raise NotImplementedError(\n        "Connect your agent: replace task with an async function that accepts lens.Case "\n        "and returns lens.Run with its trace_id or session.id."\n    )`;
  return `from datetime import timedelta

from lens import Case, Eval, Gate, Run${usesJudge ? ", judge" : ""}${usesBuiltIn ? ", scorers" : ""}
${taskImport}

async def task(case: Case) -> Run:
${taskBody}

evaluation = Eval(
    ${pyString(definition.name)},
    task=task,
    data=${pyString(`${target.dataset}@${target.revision}`)},
    scores=[${definition.spec.scorers.map(scorerCall).join(", ")}],
    baseline=${pyString(definition.spec.baseline)},
    trials=${definition.spec.trials},
    gate=${gateCall(definition)},
    timeout_per_trial=timedelta(milliseconds=${definition.spec.timeout_per_trial_ms}),
)
`;
}
