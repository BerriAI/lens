use std::collections::BTreeSet;
use url::Url;

use crate::{Error, Result, model::Report};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassRateConfidence {
    pub lower: f64,
    pub upper: f64,
}

pub fn pass_rate_confidence(passed: usize, total: usize) -> Result<PassRateConfidence> {
    if total == 0 || passed > total {
        return Err(Error::Infrastructure("Lens returned invalid case counts"));
    }
    let n = total as f64;
    let rate = passed as f64 / n;
    let z = 1.959963984540054_f64;
    let denominator = 1.0 + z * z / n;
    let center = (rate + z * z / (2.0 * n)) / denominator;
    let margin = z * ((rate * (1.0 - rate) + z * z / (4.0 * n)) / n).sqrt() / denominator;
    Ok(PassRateConfidence {
        lower: (center - margin).max(0.0),
        upper: (center + margin).min(1.0),
    })
}

pub fn marker(name: &str) -> String {
    format!("<!-- lens:{} -->", safe_text(name))
}

pub fn safe_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
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
    Ok(if !summary.gate.passed {
        "failure"
    } else if report.run.pr.is_some() && summary.baseline_run_id.is_none() {
        "neutral"
    } else {
        "success"
    })
}

pub fn markdown(report: &Report) -> Result<String> {
    let summary = report.summary()?;
    let confidence = pass_rate_confidence(summary.passed, summary.total)?;
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
    let confidence_before = baseline
        .map(|summary| pass_rate_confidence(summary.passed, summary.total))
        .transpose()?;
    let interval = |value: PassRateConfidence| {
        format!("{:.1}% to {:.1}%", value.lower * 100.0, value.upper * 100.0)
    };
    let baseline_link = report.baseline.as_ref().map_or_else(
        || "unavailable".to_owned(),
        |run| link(&safe_text(&run.version), &run.url),
    );
    let header = format!(
        "{}\n### Lens / {}\n{} · gate {} · {} regressions · {} fixed\n\n| Metric | Before / main | After / PR |\n|---|---:|---:|\n| Commit / run | {baseline_link} | {} |\n| Cases passed | {before} | {}/{} |\n| Pass rate | {} | {:.1}% |\n| 95% confidence interval | {} | {} |\n| Confidence score (95% CI lower bound) | {} | {:.1}% |\n| Uploaded trials | {} | {} |\n| Cost per case | {cost_before} | ${:.4} |",
        marker(&report.run.eval),
        safe_text(&report.run.eval),
        conclusion(report)?,
        if summary.gate.passed {
            "passed"
        } else {
            "failed"
        },
        summary.regressions.len(),
        summary.fixed.len(),
        link(&safe_text(&report.run.version), &report.run.url),
        summary.passed,
        summary.total,
        baseline.map_or_else(
            || "unavailable".into(),
            |value| format!("{:.1}%", value.pass_rate * 100.0)
        ),
        summary.pass_rate * 100.0,
        confidence_before.map_or_else(|| "unavailable".into(), interval),
        interval(confidence),
        confidence_before.map_or_else(
            || "unavailable".into(),
            |value| format!("{:.1}%", value.lower * 100.0)
        ),
        confidence.lower * 100.0,
        report.baseline.as_ref().map_or_else(
            || "unavailable".into(),
            |run| run.received_trials.to_string()
        ),
        report.run.received_trials,
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
        .then_some("No comparable baseline is available".to_owned());
    let comparison = baseline.map(|value| format!(
        "Pass-rate change: {:+.1} percentage points. Lens selected a baseline with the same dataset revision and scorers",
        100.0 * (summary.pass_rate - value.pass_rate)
    ));
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
        .chain(comparison)
        .chain([String::from("\nConfidence uses the [95% Wilson interval](https://itl.nist.gov/div898/handbook/prc/section2/prc241.htm) over case verdicts, treating cases as independent observations. Repeats are not counted as new cases. The score is its lower bound, not a probability of correctness or proof of improvement. A curated test set may not represent production traffic")])
        .chain(reasons)
        .chain(regressions)
        .chain([String::new(), link("View in Lens", &report.run.url), link("Workflow logs", &report.run.ci_url)])
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
