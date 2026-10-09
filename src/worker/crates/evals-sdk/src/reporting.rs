use std::collections::BTreeSet;
use url::Url;

use crate::{Result, model::Report};

pub fn safe_text(value: &str) -> String {
    value
        .replace(['\n', '\r'], " ")
        .replace('|', "\\|")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('@', "&#64;")
        .replace('[', "\\[")
        .replace(']', "\\]")
}

pub fn link(label: &str, value: &str) -> String {
    match Url::parse(value) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            format!(
                "[{label}]({})",
                value
                    .replace('(', "%28")
                    .replace(')', "%29")
                    .replace(['\n', '\r'], "")
            )
        }
        _ => safe_text(label),
    }
}

pub fn conclusion(report: &Report) -> Result<&'static str> {
    let summary = report.summary()?;
    Ok(
        if report.run.pr.is_some() && summary.baseline_run_id.is_none() {
            "neutral"
        } else if summary.gate.passed {
            "success"
        } else {
            "failure"
        },
    )
}

pub fn markdown(report: &Report) -> Result<String> {
    let summary = report.summary()?;
    let baseline = report
        .baseline
        .as_ref()
        .and_then(|run| run.summary.as_ref());
    let before = baseline
        .map(|value| format!("{}/{}", value.passed, value.total))
        .unwrap_or_else(|| "unavailable".into());
    let cost_before = baseline
        .map(|value| format!("${:.4}", value.cost_per_case))
        .unwrap_or_else(|| "unavailable".into());
    let header = format!(
        "<!-- lens:{} -->\n### Lens / {}\n{} · gate {} · {} regressions · {} fixed\n\n| Metric | Baseline | Candidate |\n|---|---:|---:|\n| Cases passed | {before} | {}/{} |\n| Cost per case | {cost_before} | ${:.4} |",
        report.run.eval,
        report.run.eval,
        conclusion(report)?,
        if summary.gate.passed {
            "passed"
        } else {
            "failed"
        },
        summary.regressions.len(),
        summary.fixed.len(),
        summary.passed,
        summary.total,
        summary.cost_per_case
    );
    let scores = summary.scores.iter().map(|(name, score)| {
        format!(
            "| {} | {} | {score:.3} |",
            safe_text(name),
            baseline
                .and_then(|value| value.scores.get(name))
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unavailable".into())
        )
    });
    let missing = summary
        .baseline_run_id
        .is_none()
        .then_some("No comparable baseline is available.".to_owned());
    let reasons = summary
        .gate
        .reasons
        .iter()
        .map(|reason| format!("- {}", safe_text(reason)));
    let regressions = summary.regressions.iter().map(|case| {
        format!(
            "- {}{}: {} / {}",
            if case.critical { "Critical: " } else { "" },
            safe_text(&case.title),
            link("baseline", &case.baseline_url),
            link("candidate", &case.candidate_url)
        )
    });
    Ok(std::iter::once(header)
        .chain(scores)
        .chain([String::new()])
        .chain(missing)
        .chain(reasons)
        .chain(regressions)
        .chain([String::new(), link("View in Lens", &report.run.url)])
        .collect::<Vec<_>>()
        .join("\n"))
}

pub fn terminal(report: &Report) -> Result<String> {
    let summary = report.summary()?;
    let cases = report
        .trials
        .iter()
        .map(|trial| &trial.case_id)
        .collect::<BTreeSet<_>>();
    let rows = cases
        .iter()
        .map(|id| {
            format!(
                "{id}  {} submitted  {} task errors",
                report
                    .trials
                    .iter()
                    .filter(|trial| &trial.case_id == *id)
                    .count(),
                report
                    .trials
                    .iter()
                    .filter(|trial| &trial.case_id == *id && trial.result.error.is_some())
                    .count()
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let table = if cases.is_empty() {
        String::new()
    } else {
        format!("Case / uploaded trials / task errors (verdicts below come from Lens)\n{rows}\n\n")
    };
    Ok(format!(
        "{table}{}: {}/{} passed; {} regressions; {} trial errors; ${:.4}/case; gate {}\n{}\n{}",
        report.run.eval,
        summary.passed,
        summary.total,
        summary.regressions.len(),
        summary.errors,
        summary.cost_per_case,
        if summary.gate.passed {
            "passed"
        } else {
            "failed"
        },
        summary.gate.reasons.join("\n"),
        report.run.url
    ))
}
