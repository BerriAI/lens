from typing import Final
from urllib.parse import urlsplit

from .models import Report


def safe_text(value: str) -> str:
    return (
        value.replace("\n", " ")
        .replace("\r", " ")
        .replace("|", "\\|")
        .replace("<", "&lt;")
        .replace(">", "&gt;")
        .replace("@", "&#64;")
        .replace("[", "\\[")
        .replace("]", "\\]")
    )


def link(label: str, url: str) -> str:
    parts: Final = urlsplit(url)
    if parts.scheme not in {"http", "https"} or not parts.hostname or parts.username or parts.password:
        return safe_text(label)
    escaped: Final = url.replace("(", "%28").replace(")", "%29").replace("\n", "").replace("\r", "")
    return f"[{safe_text(label)}]({escaped})"


def conclusion(report: Report) -> str:
    if report.run.pr is not None and report.summary.baseline_run_id is None:
        return "neutral"
    return "success" if report.summary.gate.passed else "failure"


def markdown(report: Report) -> str:
    summary: Final = report.summary
    baseline: Final = report.baseline.summary if report.baseline is not None else None
    before: Final = f"{baseline.passed}/{baseline.total}" if baseline else "unavailable"
    cost_before: Final = f"${baseline.cost_per_case:.4f}" if baseline else "unavailable"
    rows: Final = (
        f"<!-- lens:{report.run.eval} -->",
        f"### Lens / {report.run.eval}",
        f"{conclusion(report)} · gate {'passed' if summary.gate.passed else 'failed'} · "
        f"{len(summary.regressions)} regressions · {len(summary.fixed)} fixed",
        "",
        "| Metric | Baseline | Candidate |",
        "|---|---:|---:|",
        f"| Cases passed | {before} | {summary.passed}/{summary.total} |",
        f"| Cost per case | {cost_before} | ${summary.cost_per_case:.4f} |",
    )
    scores: Final = tuple(
        f"| {safe_text(name)} | {baseline.scores.get(name, 'unavailable') if baseline else 'unavailable'} | "
        f"{value:.3f} |"
        for name, value in summary.scores.items()
    )
    reasons: Final = tuple(f"- {safe_text(reason)}" for reason in summary.gate.reasons)
    regressions: Final = tuple(
        f"- {'Critical: ' if case.critical else ''}{safe_text(case.title)}: "
        f"{link('baseline', case.baseline_url)} / {link('candidate', case.candidate_url)}"
        for case in summary.regressions
    )
    missing: Final = ("No comparable baseline is available.",) if summary.baseline_run_id is None else ()
    return "\n".join((*rows, *scores, "", *missing, *reasons, *regressions, "", link("View in Lens", report.url)))


def trial_table(report: Report) -> str:
    if not report.trials:
        return ""
    case_ids: Final = tuple(dict.fromkeys(trial.case_id for trial in report.trials))
    rows: Final = tuple(
        f"{case_id}  "
        f"{sum(trial.case_id == case_id for trial in report.trials)} submitted  "
        f"{sum(trial.case_id == case_id and trial.result.error is not None for trial in report.trials)} task errors"
        for case_id in case_ids
    )
    return "Case / uploaded trials / task errors (verdicts below come from Lens)\n" + "\n".join(rows) + "\n\n"


def terminal(report: Report) -> str:
    result: Final = report.summary
    return trial_table(report) + (
        f"{report.run.eval}: {result.passed}/{result.total} passed; "
        f"{len(result.regressions)} regressions; {result.errors} trial errors; "
        f"${result.cost_per_case:.4f}/case; gate {'passed' if result.gate.passed else 'failed'}\n"
        + "\n".join(result.gate.reasons)
        + f"\n{report.url}"
    )
