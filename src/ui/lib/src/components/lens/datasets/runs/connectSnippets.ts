export interface ConnectTarget {
  readonly agent: string;
  readonly dataset: string;
  readonly revision: number;
  readonly baseUrl: string;
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
project = ${JSON.stringify(target.agent)}
evals = "evals/"
base_url = ${JSON.stringify(target.baseUrl)}
`;
}

export function evalSnippet(target: ConnectTarget): string {
  return `from lens import Eval, Gate, Run, scorers


async def task(case):
    session_id = await start_agent_run(case.input)  # your agent's API
    return Run(trace={"session.id": session_id})


evaluation = Eval(
    ${JSON.stringify(target.agent)},
    task=task,
    data="${target.dataset}@${target.revision}",
    scores=[scorers.task_completed()],
    baseline="main",
    gate=Gate(regressions=0, critical=0),
)
`;
}

export function githubAppInstallUrl(slug: string | undefined): string | null {
  const trimmed = slug?.trim();
  return trimmed
    ? `https://github.com/apps/${encodeURIComponent(trimmed)}/installations/new`
    : null;
}
