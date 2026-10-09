import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Final

REPO: Final = Path(__file__).resolve().parents[3]
MUTATIONS: Final = (
    ("engine.rs", "buffer_unordered(spec.concurrency)", "buffer_unordered(1)", "configured concurrency"),
    ("engine.rs", "0..spec.trials", "0..1", "all trials executed"),
    ("engine.rs", "gate: spec.gate.clone()", "gate: Gate::default()", "configured gate forwarded"),
    ("engine.rs", "execution.identity, fingerprint", '"constant", fingerprint', "distinct executions at one SHA"),
    ("model.rs", "ids.contains(&case.id)", "ids.is_empty()", "case subset selection"),
    ("client.rs", "if retry { self.attempts } else { 1 }", "if retry { 1 } else { self.attempts }", "bounded retries"),
    ("client.rs", 'code == "run_closed"', 'code == "never"', "lost finish acknowledgement"),
    (
        "client.rs",
        "data.dataset_id != dataset.id || data.revision != dataset.revision",
        "false",
        "dataset identity checked",
    ),
    ("devserver.rs", "> spec.trials,", ">= spec.trials,", "majority ties fail"),
    (
        "devserver.rs",
        "Some(&true) && !verdicts[&case.id]",
        "Some(&false) && !verdicts[&case.id]",
        "individual regressions detected",
    ),
    ("devserver.rs", "passed: failures.is_empty()", "passed: !failures.is_empty()", "server gate conjunction"),
    (
        "reporting.rs",
        "summary.baseline_run_id.is_none()",
        "summary.baseline_run_id.is_some()",
        "missing baseline check is neutral",
    ),
    (
        "github.rs",
        'comment.user.login == "github-actions[bot]"',
        'comment.user.login != "github-actions[bot]"',
        "only bot comments edited",
    ),
    ("setup.rs", "if file.exists()", "if false", "existing setup files protected"),
)


def check(root: Path, mutation: tuple[str, str, str, str]) -> dict[str, str | bool]:
    file, before, after, name = mutation
    target: Final = root / "runtime/crates/evals-sdk/src" / file
    original: Final = target.read_text()
    if before not in original:
        return {"name": name, "killed": False, "error": "Mutation anchor missing"}
    target.write_text(original.replace(before, after, 1))
    try:
        result: Final = subprocess.run(
            ["cargo", "test", "--manifest-path", "runtime/Cargo.toml", "-p", "lens-evals-sdk", "--tests", "--quiet"],
            cwd=root,
            env={**os.environ, "CARGO_TARGET_DIR": str(REPO / "runtime/target/mutations")},
            capture_output=True,
            text=True,
            timeout=180,
        )
        return {"name": name, "killed": result.returncode == 101 and "test result: FAILED" in result.stdout}
    finally:
        target.write_text(original)


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="lens-native-mutations-") as directory:
        root: Final = Path(directory)
        (root / "runtime").mkdir()
        shutil.copytree(REPO / "runtime/crates/evals-sdk", root / "runtime/crates/evals-sdk")
        shutil.copytree(REPO / "packages/sdk/tests/fixtures", root / "packages/sdk/tests/fixtures")
        manifest: Final = (
            (REPO / "runtime/Cargo.toml")
            .read_text()
            .replace('members = ["crates/*"]', 'members = ["crates/evals-sdk"]')
        )
        (root / "runtime/Cargo.toml").write_text(manifest)
        shutil.copyfile(REPO / "runtime/Cargo.lock", root / "runtime/Cargo.lock")
        baseline: Final = subprocess.run(
            ["cargo", "test", "--manifest-path", "runtime/Cargo.toml", "-p", "lens-evals-sdk", "--tests", "--quiet"],
            cwd=root,
            env={**os.environ, "CARGO_TARGET_DIR": str(REPO / "runtime/target/mutations")},
            capture_output=True,
            text=True,
        )
        if baseline.returncode:
            raise RuntimeError(baseline.stderr + baseline.stdout)
        results: Final = tuple(check(root, mutation) for mutation in MUTATIONS)
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
