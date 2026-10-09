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

fn checked_link(label: &str, value: &str) -> Option<String> {
    match Url::parse(value) {
        Ok(url)
            if matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            Some(format!(
                "[{label}]({})",
                value
                    .replace('(', "%28")
                    .replace(')', "%29")
                    .replace(['\n', '\r'], "")
            ))
        }
        _ => None,
    }
}

pub fn link(label: &str, value: &str) -> String {
    checked_link(label, value).unwrap_or_else(|| safe_text(label))
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

const LOGO: &str = concat!(
    "<picture><source media=\"(prefers-color-scheme: dark)\" srcset=\"",
    "https://raw.githubusercontent.com/BerriAI/lens/c33720fd5e5b54de40911ec0cd27ac9402b9b1e1/assets/lens-logo-dark.png\">",
    "<img src=\"https://raw.githubusercontent.com/BerriAI/lens/c33720fd5e5b54de40911ec0cd27ac9402b9b1e1/assets/lens-logo-light.png\" ",
    "alt=\"Lens\" width=\"108\" height=\"30\"></picture>"
);

fn cost(value: f64) -> String {
    format!("${:.4}", if value == 0.0 { 0.0 } else { value })
}

fn build_link(run: &crate::model::EvalRun) -> String {
    link(
        &safe_text(&run.version.chars().take(7).collect::<String>()),
        &run.url,
    )
}

pub fn markdown(report: &Report) -> Result<String> {
    let summary = report.summary()?;
    let confidence = pass_rate_confidence(summary.passed, summary.total)?;
    let baseline = report
        .baseline
        .as_ref()
        .and_then(|run| run.summary.as_ref());
    let baseline_confidence = baseline
        .map(|value| pass_rate_confidence(value.passed, value.total))
        .transpose()?;
    let before = |metric: fn(&crate::model::Summary) -> String| {
        baseline.map(metric).unwrap_or_else(|| "unavailable".into())
    };
    let interval = |value: PassRateConfidence| {
        format!("{:.1}% to {:.1}%", value.lower * 100.0, value.upper * 100.0)
    };
    let gate = if summary.gate.passed {
        "passed"
    } else {
        "failed"
    };
    let header = format!(
        "{}\n{LOGO}\n\n### {}\n\n**Confidence {:.1}/5** · **Passed {}** · **Failed {}** · Trial errors {}\n\nGate {gate} · {} regressions · {} fixed · {} cases\n\n| Metric | Before / main | After / PR |\n|---|---:|---:|\n| Build | {} | {} |\n| Passed / total | {} | {}/{} |\n| Failed | {} | {} |\n| Pass rate | {} | {:.1}% |",
        marker(&report.run.eval),
        safe_text(&report.run.eval),
        confidence.lower * 5.0,
        summary.passed,
        summary.total - summary.passed,
        summary.errors,
        summary.regressions.len(),
        summary.fixed.len(),
        summary.total,
        report
            .baseline
            .as_ref()
            .map(build_link)
            .unwrap_or_else(|| "unavailable".into()),
        build_link(&report.run),
        before(|value| format!("{}/{}", value.passed, value.total)),
        summary.passed,
        summary.total,
        before(|value| (value.total - value.passed).to_string()),
        summary.total - summary.passed,
        before(|value| format!("{:.1}%", value.pass_rate * 100.0)),
        summary.pass_rate * 100.0
    );
    let comparison = baseline.map_or_else(
        || "No comparable baseline is available".to_owned(),
        |value| {
            format!(
                "Pass-rate change: {:+.1} percentage points",
                100.0 * (summary.pass_rate - value.pass_rate)
            )
        },
    );
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
    let footer = std::iter::once(link("View results in Lens", &report.run.url))
        .chain(checked_link("Workflow logs", &report.run.ci_url))
        .collect::<Vec<_>>()
        .join(" · ");
    let details = format!(
        "<details>\n<summary>Benchmark details and confidence</summary>\n\nConfidence is the lower bound of the [95% Wilson interval](https://itl.nist.gov/div898/handbook/prc/section2/prc241.htm), scaled to 5. A small test set gives a conservative score even when every case passes. It is not a probability of correctness or proof of improvement\n\nCase verdicts are treated as independent observations. Repeats are not counted as new cases. A curated test set may not represent production traffic\n\n| Metric | Before | After |\n|---|---:|---:|\n| 95% confidence interval | {} | {} |\n| Confidence / 5 | {} | {:.1} |\n| Uploaded trials | {} | {} |\n| Trial errors | {} | {} |\n| Cost per case | {} | {} |",
        baseline_confidence
            .map(interval)
            .unwrap_or_else(|| "unavailable".into()),
        interval(confidence),
        baseline_confidence
            .map(|value| format!("{:.1}", value.lower * 5.0))
            .unwrap_or_else(|| "unavailable".into()),
        confidence.lower * 5.0,
        report
            .baseline
            .as_ref()
            .map(|run| run.received_trials.to_string())
            .unwrap_or_else(|| "unavailable".into()),
        report.run.received_trials,
        before(|value| value.errors.to_string()),
        summary.errors,
        before(|value| cost(value.cost_per_case)),
        cost(summary.cost_per_case)
    );
    let scores = summary.scores.iter().map(|(name, score)| {
        format!(
            "| {} | {} | {score:.3} |",
            safe_text(name),
            baseline
                .and_then(|value| value.scores.get(name))
                .map(|value| format!("{value:.3}"))
                .unwrap_or_else(|| "unavailable".into())
        )
    });
    Ok(std::iter::once(header)
        .chain([String::new(), comparison, String::new()])
        .chain(reasons)
        .chain(regressions)
        .chain([String::new(), footer, String::new(), details])
        .chain(scores)
        .chain([String::new(), String::from("</details>")])
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
        "{table}{}: {}/{} passed; {} regressions; {} trial errors; {}/case; gate {}\n{}\n{}",
        report.run.eval,
        summary.passed,
        summary.total,
        summary.regressions.len(),
        summary.errors,
        cost(summary.cost_per_case),
        if summary.gate.passed {
            "passed"
        } else {
            "failed"
        },
        summary.gate.reasons.join("\n"),
        report.run.url
    ))
}
