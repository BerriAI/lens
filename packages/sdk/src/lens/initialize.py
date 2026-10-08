import json
import re
from pathlib import Path
from typing import Final

from .config import Settings
from .errors import ConfigurationError

ACTION: Final = "BerriAI/litellm-lens/packages/sdk/action@litellm_eval_sdk"


def workflow(action: str) -> str:
    return f"""name: Lens
on:
  push:
    branches: [main]
  pull_request:
permissions:
  contents: read
  checks: write
  pull-requests: write
concurrency:
  group: lens-${{{{ github.event.pull_request.number || github.ref }}}}
  cancel-in-progress: true
jobs:
  eval:
    if: github.event_name != 'pull_request' || github.event.pull_request.head.repo.full_name == github.repository
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
      - uses: actions/setup-python@a26af69be951a213d495a4c3e4e4022e16d87065
        with:
          python-version: '3.11'
      - run: python -m pip install -e .
      - uses: {action}
        with:
          api-key: ${{{{ secrets.LENS_API_KEY }}}}
          base-url: ${{{{ vars.LENS_BASE_URL }}}}
"""


def initialize(root: Path, dataset: str, project: str, action: str = ACTION) -> None:
    name: Final = dataset.split("@", 1)[0]
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*(?:@[1-9][0-9]*)?", dataset):
        raise ConfigurationError("Use a lowercase dataset name or name@positive-revision")
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_./-]+@[A-Za-z0-9_.-]+", action):
        raise ConfigurationError("Action reference must have the form owner/repo[/path]@ref")
    config: Final = root / "pyproject.toml"
    settings: Final = Settings.load(root)
    target: Final = root / settings.evals / f"{name.replace('-', '_')}.py"
    workflow_file: Final = root / ".github/workflows/lens.yml"
    for file in (target, workflow_file):
        if file.exists():
            raise ConfigurationError(f"Refusing to overwrite {file}")
    if not settings.project and not project:
        raise ConfigurationError("Pass --project with your agent.name")
    existing: Final = config.read_text() if config.exists() else ""
    if not settings.project and "[tool.lens]" in existing:
        raise ConfigurationError("Set project in the existing [tool.lens] table before initializing")
    content: Final = f"""from lens import Case, Eval, Gate, Run, scorers


async def task(case: Case) -> Run:
    raise NotImplementedError("Await your agent, then return Run(trace={{'session.id': session_id}})")


evaluation = Eval(
    {json.dumps(name)},
    task=task,
    data={json.dumps(dataset)},
    scores=[scorers.task_completed()],
    trials=3,
    gate=Gate(pass_rate=1),
)
"""
    target.parent.mkdir(parents=True, exist_ok=True)
    workflow_file.parent.mkdir(parents=True, exist_ok=True)
    if not settings.project:
        config.write_text(existing + f'\n[tool.lens]\nproject = {json.dumps(project)}\nevals = "evals/"\n')
    target.write_text(content)
    workflow_file.write_text(workflow(action))
    print(f"Created {target} and {workflow_file}. Implement task() and install your agent dependencies in the workflow")
    print("Set LENS_BASE_URL and LENS_API_KEY locally, and the matching GitHub variable/secret for CI")
