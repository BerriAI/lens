import argparse
import asyncio
import contextlib
import json
import sys
from pathlib import Path
from typing import Final

from pydantic import ValidationError

from .config import Execution, Settings
from .discovery import discover
from .errors import LensError
from .evaluation import Eval
from .initialize import ACTION
from .models import Report
from .reporting import terminal


def parser() -> argparse.ArgumentParser:
    root: Final = argparse.ArgumentParser(prog="lens")
    commands: Final = root.add_subparsers(dest="command", required=True)
    evaluate: Final = commands.add_parser("eval")
    evaluate.add_argument("path", nargs="?")
    evaluate.add_argument("--eval", dest="name")
    evaluate.add_argument("--ci", action="store_true")
    evaluate.add_argument("--json", action="store_true", dest="json_output")
    initialize: Final = commands.add_parser("init")
    initialize.add_argument("dataset")
    initialize.add_argument("--project", default="")
    initialize.add_argument("--action-ref", default=ACTION)
    dev: Final = commands.add_parser("dev-server")
    dev.add_argument("--host", default="127.0.0.1")
    dev.add_argument("--port", type=int, default=8765)
    dev.add_argument("--dataset-file", type=Path)
    return root


async def run_one(evaluation: Eval, settings: Settings, execution: Execution) -> Report | LensError:
    try:
        return await evaluation.arun(settings=settings, execution=execution)
    except LensError as error:
        return error


async def evaluate(args: argparse.Namespace) -> int:
    settings: Final = Settings.load(Path.cwd())
    execution: Final = Execution.github() if args.ci else Execution.local()
    with contextlib.redirect_stdout(sys.stderr):
        evaluations: Final = discover(Path(args.path or settings.evals), args.name)
        outcomes: Final = tuple([await run_one(evaluation, settings, execution) for evaluation in evaluations])
    reports: Final = tuple(outcome for outcome in outcomes if isinstance(outcome, Report))
    errors: Final = tuple(outcome for outcome in outcomes if isinstance(outcome, LensError))
    for error in errors:
        print(f"Lens: {error}", file=sys.stderr)
    if args.json_output:
        print(json.dumps({"runs": [report.run.model_dump(mode="json") for report in reports]}))
    else:
        for report in reports:
            print(terminal(report))
    return 2 if errors else (0 if all(report.summary.gate.passed for report in reports) else 1)


def main(argv: list[str] | None = None) -> int:
    args: Final = parser().parse_args(argv)
    try:
        if args.command == "eval":
            return asyncio.run(evaluate(args))
        if args.command == "init":
            from .initialize import initialize

            initialize(Path.cwd(), args.dataset, args.project, args.action_ref)
            return 0
        from .devserver import serve

        serve(args.host, args.port, args.dataset_file)
        return 0
    except (LensError, ValidationError, OSError, ValueError) as error:
        print(f"Lens: {error}", file=sys.stderr)
        if getattr(args, "json_output", False):
            print('{"runs": []}')
        return 2
    except KeyboardInterrupt:
        print("Lens: interrupted", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
