import json
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
from typing import Final

SDK: Final = Path(__file__).resolve().parents[1]
MUTATIONS: Final = (
    ("client.py", "and can_retry:", "and False:", "retry HTTP errors"),
    ("client.py", 'error.status == 409 and error.code == "run_closed"', "False", "recover finish acknowledgement"),
    ("evaluation.py", "asyncio.Semaphore(self.concurrency)", "asyncio.Semaphore(1)", "configured concurrency"),
    ("evaluation.py", "product(cases, range(self.trials))", "product(cases, range(1))", "all trials executed"),
    ("evaluation.py", "gate=self.threshold,", "gate=Gate(),", "configured server gate"),
    (
        "evaluation.py",
        'key: Final = f"{self.name}:{execution.version}:{execution.identity}:{fingerprint}"',
        'key: Final = f"{self.name}:{execution.version}:{fingerprint}"',
        "distinct CI executions",
    ),
    ("evaluation.py", "self.case_ids is None or case.id in self.case_ids", "True", "subset selection"),
    ("evaluation.py", "cost_usd=value.cost_usd,", "cost_usd=0,", "cost forwarded"),
    ("evaluation.py", 'if current.status == "failed":', "if False:", "server failure classification"),
    ("models.py", "if not self.summary.gate.passed:", "if self.summary.gate.passed:", "assert gate verdict"),
    (
        "models.py",
        'if len(self.trace) != 1 or next(iter(self.trace)) not in {"session.id", "trace_id"}:',
        "if len(self.trace) == 0:",
        "valid trace references",
    ),
    ("devserver.py", "> spec.trials / 2", ">= spec.trials / 2", "ties fail"),
    ("devserver.py", "prior.get(case.id) is True and not verdicts[case.id]", "False", "regressions detected"),
    ("devserver.py", "passed=not failures", "passed=bool(failures)", "server gate conjunction"),
    ("devserver.py", 'if run.status != "running":', "if False:", "closed result rejection"),
    (
        "reporting.py",
        "report.summary.baseline_run_id is None",
        "report.summary.baseline_run_id is not None",
        "missing baseline neutral",
    ),
    ("cli.py", "return 2 if errors else", "return 0 if errors else", "infrastructure exit code"),
    ("cli.py", "for report in reports) else 1", "for report in reports) else 0", "gate failure exit code"),
    (
        "github.py",
        'comment.user.get("login") == "github-actions[bot]"',
        'comment.user.get("login") != "github-actions[bot]"',
        "only edit owned comment",
    ),
    ("discovery.py", "if len(set(names)) != len(names):", "if False:", "duplicate evaluation rejection"),
)


def check(mutation: tuple[str, str, str, str]) -> dict[str, str | bool]:
    file, before, after, name = mutation
    with tempfile.TemporaryDirectory(prefix="lens-mutation-") as directory:
        root: Final = Path(directory)
        shutil.copytree(SDK / "src", root / "src")
        shutil.copytree(SDK / "tests", root / "tests", ignore=shutil.ignore_patterns("__pycache__"))
        shutil.copyfile(SDK / "pyproject.toml", root / "pyproject.toml")
        target: Final = root / "src/lens" / file
        original: Final = target.read_text()
        if before not in original:
            return {"name": name, "killed": False, "error": "Mutation anchor missing"}
        target.write_text(original.replace(before, after, 1))
        result: Final = subprocess.run(
            [sys.executable, "-m", "pytest", "tests", "-q", "--tb=no"],
            cwd=root,
            env={**os.environ, "PYTHONPATH": str(root / "src")},
            capture_output=True,
            text=True,
            timeout=60,
        )
        return {"name": name, "killed": result.returncode == 1 and "failed" in result.stdout}


def main() -> int:
    with ThreadPoolExecutor(max_workers=2) as executor:
        results: Final = tuple(executor.map(check, MUTATIONS))
    print(
        json.dumps(
            {
                "killed": sum(result["killed"] is True for result in results),
                "total": len(results),
                "mutations": results,
            },
            indent=2,
        )
    )
    return 0 if all(result["killed"] for result in results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
