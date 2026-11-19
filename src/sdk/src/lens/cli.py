import asyncio
import contextlib
import json
import os
import sys
from pathlib import Path
from typing import Final, Literal

from pydantic import BaseModel, ConfigDict, Field, ValidationError

from . import _native, local
from .client import Lens
from .config import Execution, Settings
from .discovery import discover
from .errors import LensError
from .evaluation import Eval
from .models import Report
from .reporting import terminal


class Arguments(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    command: str
    path: str | None = None
    name: str | None = None
    eval_name: str | None = None
    ci: bool = False
    fast: bool = False
    tasks: str | None = None
    config: str | None = None
    concurrency: int = 16
    run_id: str | None = None
    only: tuple[str, ...] = ()
    json_output: bool = Field(default=False, alias="json")
    code: int = 0
    text: str = ""


class Check(BaseModel):
    model_config = ConfigDict(frozen=True)
    name: str
    ok: bool
    detail: str


class Diagnosis(BaseModel):
    model_config = ConfigDict(frozen=True)
    ok: bool
    checks: tuple[Check, ...]


async def run_one(evaluation: Eval, settings: Settings, execution: Execution) -> Report | LensError:
    try:
        return await evaluation.arun(settings=settings, execution=execution)
    except LensError as error:
        return error


async def evaluate(args: Arguments, mode: Literal["eval", "doctor"]) -> int:
    settings: Final = Settings.load(Path.cwd())
    if args.eval_name is not None:
        execution: Final = Execution.named(ci=args.ci)
        report: Final = await Lens(settings.endpoint(), os.environ.get("LENS_API_KEY", "")).evals.arun(
            args.eval_name, execution=execution
        )
        return display((report,), args)
    with contextlib.redirect_stdout(sys.stderr):
        evaluations: Final = discover(Path(args.path or settings.evals), args.name)
    if mode == "doctor":
        diagnosis: Final = Diagnosis.model_validate_json(
            await _native.diagnose("[" + ",".join(evaluation._spec() for evaluation in evaluations) + "]", args.ci)
        )
        if args.json_output:
            print(diagnosis.model_dump_json())
        else:
            for check in diagnosis.checks:
                print(f"{'OK' if check.ok else 'ERROR'} {check.name}: {check.detail}")
            print(
                "No agent tasks were called. Run lens eval when ready"
                if diagnosis.ok
                else "Fix these issues before running evals"
            )
        return 0 if diagnosis.ok else 2
    with contextlib.redirect_stdout(sys.stderr):
        local_execution: Final = Execution.github() if args.ci else Execution.local()
        outcomes: Final = tuple([await run_one(evaluation, settings, local_execution) for evaluation in evaluations])
    return display(outcomes, args)


def display(outcomes: tuple[Report | LensError, ...], args: Arguments) -> int:
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


def run_local(args: Arguments) -> int:
    if args.tasks is None or args.config is None:
        raise LensError("lens run needs a tasks file and --config")
    command: Final = " ".join(
        ("lens run", args.tasks, "--config", args.config, *(f"--only {key}" for key in args.only))
    )
    asyncio.run(local.run(command, Path(args.config), args.concurrency, args.fast, tasks=args.tasks, only=args.only))
    return 0


def resume_local(args: Arguments) -> int:
    if args.run_id is None:
        raise LensError("lens resume needs a run id")
    meta: Final = RunMeta.model_validate_json(_native.run_meta(args.run_id))
    command: Final = f"lens resume {args.run_id}"
    asyncio.run(local.run(command, Path(meta.config_file), args.concurrency, args.fast, resume=args.run_id))
    return 0


class RunMeta(BaseModel):
    model_config = ConfigDict(frozen=True, extra="ignore")
    config_file: str


def main(argv: list[str] | None = None) -> int:
    raw: Final = _native.parse_cli(["lens", *(sys.argv[1:] if argv is None else argv)])
    args: Final = Arguments.model_validate_json(raw)
    try:
        if args.command == "help":
            print(args.text, file=sys.stderr if args.code else sys.stdout, end="")
            return args.code
        if args.command == "eval":
            return asyncio.run(evaluate(args, "eval"))
        if args.command == "doctor":
            return asyncio.run(evaluate(args, "doctor"))
        if args.command == "demo":
            _native.demo(args.fast)
            return 0
        if args.command == "run":
            return run_local(args)
        if args.command == "resume":
            return resume_local(args)
        if args.command == "show":
            _native.show(args.run_id)
            return 0
        print(_native.control(raw))
        return 0
    except (LensError, ValidationError, OSError, ValueError) as error:
        print(f"Lens: {error}", file=sys.stderr)
        if args.json_output:
            print('{"runs": []}' if args.command == "eval" else '{"ok": false, "checks": []}')
        return 2
    except KeyboardInterrupt:
        print("Lens: interrupted", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
