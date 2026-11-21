use std::time::Duration;

use crate::demo::style::{
    ACCENT, Line, MUTED, Style, TEXT, accent, bad, badge, bar, gap, good, joined, muted, pad,
    shorten, span, strong, text, thousands,
};
use crate::demo::{body, brand, detail, result, spinner};
use crate::local::{Outcome, Progress, RunMeta, Target};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Count {
    pub done: u64,
    pub pass: u64,
    pub fail: u64,
    pub error: u64,
    pub cost: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Tally {
    pub targets: Vec<Target>,
    pub counts: Vec<Count>,
    pub task_count: u64,
    pub resumed: u64,
}

impl Tally {
    pub fn new(meta: &RunMeta, resumed: usize) -> Self {
        Self {
            targets: meta.targets.clone(),
            counts: vec![Count::default(); meta.targets.len()],
            task_count: meta.task_count as u64,
            resumed: resumed as u64,
        }
    }

    pub fn record(&mut self, progress: &Progress) {
        match progress {
            Progress::Started { meta, resumed } => *self = Self::new(meta, *resumed),
            Progress::Finished(attempt) => {
                let index = match self
                    .targets
                    .iter()
                    .position(|target| *target == attempt.target)
                {
                    Some(index) => index,
                    None => {
                        self.targets.push(attempt.target.clone());
                        self.counts.push(Count::default());
                        self.targets.len() - 1
                    }
                };
                let count = &mut self.counts[index];
                count.done += 1;
                count.cost += attempt.result.cost + attempt.result.judge_cost;
                match attempt.result.outcome {
                    Outcome::Pass => count.pass += 1,
                    Outcome::Fail { .. } => count.fail += 1,
                    Outcome::Error { .. } => count.error += 1,
                }
            }
        }
    }

    pub fn total(&self) -> u64 {
        self.task_count * self.targets.len() as u64
    }

    pub fn done(&self) -> u64 {
        (self.resumed + self.counts.iter().map(|count| count.done).sum::<u64>()).min(self.total())
    }

    pub fn cost(&self) -> f64 {
        self.counts.iter().map(|count| count.cost).sum()
    }

    pub fn fraction(&self) -> f64 {
        match self.total() {
            0 => 0.0,
            total => self.done() as f64 / total as f64,
        }
    }

    pub fn complete(&self) -> bool {
        self.total() > 0 && self.done() >= self.total()
    }
}

pub fn elapsed(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m {:02}s", seconds / 60, seconds % 60),
        _ => format!("{}h {:02}m", seconds / 3600, seconds % 3600 / 60),
    }
}

fn unique(names: impl Iterator<Item = String>) -> Vec<String> {
    names.fold(Vec::new(), |seen, name| {
        if seen.contains(&name) {
            seen
        } else {
            seen.into_iter().chain([name]).collect()
        }
    })
}

fn roster(
    names: Vec<String>,
    describe: impl Fn(&str) -> (brand::Brand, String),
) -> Vec<crate::demo::style::Span> {
    names
        .iter()
        .enumerate()
        .flat_map(|(index, name)| {
            let (brand, label) = describe(name);
            (index > 0)
                .then(|| gap(3))
                .into_iter()
                .chain([badge(brand), text(label)])
        })
        .collect()
}

pub fn found(tally: &Tally) -> Vec<Line> {
    let harnesses = unique(tally.targets.iter().map(|target| target.harness.clone()));
    let models = unique(tally.targets.iter().map(|target| target.model.clone()));
    let resumed = (tally.resumed > 0).then(|| {
        detail(vec![muted(format!(
            "resuming       {} cases already done",
            thousands(tally.resumed)
        ))])
    });
    [
        Line::blank(),
        result(vec![
            strong(format!("Found {} tasks", thousands(tally.task_count))),
            text(format!(" × {} configurations", tally.targets.len())),
        ]),
        detail(
            [muted("harnesses  ")]
                .into_iter()
                .chain(roster(harnesses, |id| {
                    let brand = brand::harness(id);
                    (brand, brand.label.to_owned())
                }))
                .collect(),
        ),
        detail(
            [muted("models     ")]
                .into_iter()
                .chain(roster(models, |name| (brand::model(name), name.to_owned())))
                .collect(),
        ),
    ]
    .into_iter()
    .chain(resumed)
    .chain([Line::blank()])
    .collect()
}

pub fn frame(tally: &Tally, clock: Duration, spin: usize, width: usize) -> Vec<Line> {
    let fraction = tally.fraction();
    let finished = tally.complete();
    let status = if finished {
        result(vec![good(format!(
            "{} cases complete",
            thousands(tally.done())
        ))])
    } else {
        spinner(spin, "Running cases…", &elapsed(clock), ACCENT)
    };
    let percent = format!("{:>3}%", (fraction * 100.0).floor() as u32);
    let progress = Line::indented(
        2,
        bar(fraction, width.saturating_sub(16))
            .into_iter()
            .chain([gap(2), accent(percent)])
            .collect(),
    );
    let counts = body(joined(vec![
        good(format!(
            "{} / {} complete",
            thousands(tally.done()),
            thousands(tally.total())
        )),
        good(format!("${:.2} so far, incl. judge", tally.cost())),
    ]));
    let summary = [status, Line::blank(), progress, Line::blank(), counts];
    if finished {
        return summary.into_iter().collect();
    }
    let rows = tally
        .targets
        .iter()
        .zip(&tally.counts)
        .map(|(target, count)| {
            let harness = brand::harness(&target.harness);
            let share = match tally.task_count {
                0 => 0.0,
                tasks => count.done as f64 / tasks as f64,
            };
            body(
                [
                    badge(harness),
                    span(pad(harness.label, 18), Style::fg(TEXT)),
                    badge(brand::model(&target.model)),
                    span(pad(&shorten(&target.model, 11), 12), Style::fg(MUTED)),
                ]
                .into_iter()
                .chain(bar(share, 14))
                .chain([
                    gap(2),
                    good(format!("{:>5} pass", thousands(count.pass))),
                    gap(1),
                    bad(format!("{:>4} fail", thousands(count.fail + count.error))),
                ])
                .collect(),
            )
        });
    summary
        .into_iter()
        .chain([Line::blank()])
        .chain(rows)
        .collect()
}
