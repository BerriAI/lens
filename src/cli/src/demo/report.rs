use crate::demo::brand;
use crate::demo::style::{
    ACCENT, Align, BAD, Column, FAINT, GOOD, Line, MUTED, RULE, Rgb, Span, Style, TEXT, WARN,
    accent, badge, branded, cell, faint, gap, joined, muted, pad, pad_left, shorten, span, split,
    strong, table, text, thousands, warn,
};
use crate::demo::{detail, result};
use crate::local::{Summary, TargetStats};

const VALUE: usize = 9;
const PLOT_ROWS: usize = 9;
const FIXED_COLUMNS: usize = 2 + 3 + 7 + 5 + 5 + 8 + 7 + 6;

pub fn marker_text(index: usize) -> String {
    u32::try_from(index)
        .ok()
        .filter(|index| *index < 20)
        .and_then(|index| char::from_u32(0x2460 + index))
        .map_or_else(|| format!("{}", index + 1), String::from)
}

struct Report<'a> {
    summary: &'a Summary,
    harness_width: usize,
    model_width: usize,
    width: usize,
}

impl<'a> Report<'a> {
    fn new(summary: &'a Summary, width: usize) -> Self {
        let harness_width = summary
            .stats
            .iter()
            .map(|stat| brand::harness(&stat.target.harness).label.chars().count())
            .max()
            .unwrap_or(6)
            .clamp(6, 18);
        let longest_model = summary
            .stats
            .iter()
            .map(|stat| {
                stat.target
                    .label
                    .as_deref()
                    .unwrap_or(&stat.target.model)
                    .chars()
                    .count()
            })
            .max()
            .unwrap_or(6);
        let room = width.saturating_sub(FIXED_COLUMNS + harness_width + 4 + 3);
        Self {
            summary,
            harness_width,
            model_width: longest_model.clamp(6, 18).min(room.max(6)),
            width,
        }
    }

    fn stats(&self) -> &[TargetStats] {
        &self.summary.stats
    }

    fn picked(&self, index: usize) -> bool {
        self.summary.recommended == Some(index)
    }

    fn marker_style(&self, index: usize) -> Style {
        if self.picked(index) {
            Style::fg(ACCENT).bold()
        } else {
            Style::fg(TEXT)
        }
    }

    fn bar_color(&self, index: usize) -> Rgb {
        if self.picked(index) { ACCENT } else { MUTED }
    }

    fn marker(&self, index: usize) -> Span {
        span(format!("{} ", marker_text(index)), self.marker_style(index))
    }

    fn who_width(&self) -> usize {
        2 + 2 + 2 + self.harness_width + 2 + 2 + self.model_width + 1
    }

    fn who(&self, index: usize) -> Vec<Span> {
        let target = &self.stats()[index].target;
        let harness = brand::harness(&target.harness);
        vec![
            gap(2),
            self.marker(index),
            badge(harness),
            span(
                pad(
                    &shorten(harness.label, self.harness_width),
                    self.harness_width + 2,
                ),
                Style::fg(TEXT),
            ),
            badge(brand::model(&target.model)),
            span(
                pad(
                    &shorten(
                        target.label.as_deref().unwrap_or(&target.model),
                        self.model_width,
                    ),
                    self.model_width + 1,
                ),
                Style::fg(MUTED),
            ),
        ]
    }

    fn track(&self) -> usize {
        self.width
            .saturating_sub(self.who_width() + VALUE + 1)
            .max(10)
    }

    fn bar_row(&self, index: usize, fraction: f64, value: String) -> Line {
        Line::new(
            self.who(index)
                .into_iter()
                .chain(fill(fraction, self.track(), self.bar_color(index)))
                .chain([span(pad_left(&value, VALUE), self.marker_style(index))])
                .collect(),
        )
    }

    fn label(&self, index: usize) -> String {
        let target = &self.stats()[index].target;
        if let Some(label) = &target.label {
            return label.clone();
        }
        let shared = self
            .stats()
            .iter()
            .filter(|stat| stat.target.model == target.model)
            .count()
            > 1;
        if shared {
            format!(
                "{} / {}",
                brand::harness(&target.harness).label,
                target.model
            )
        } else {
            target.model.clone()
        }
    }

    fn best(&self) -> Option<usize> {
        self.stats()
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| left.pass_rate().total_cmp(&right.pass_rate()))
            .map(|(index, _)| index)
    }

    fn recommendation(&self) -> Vec<Line> {
        let Some(pick) = self
            .summary
            .recommended
            .filter(|pick| *pick < self.stats().len())
        else {
            return vec![
                result(vec![strong("No recommendation")]),
                detail(vec![muted(
                    "A recommendation needs passing cases and complete cost metrics. Inspect the results below.",
                )]),
            ];
        };
        let chosen = &self.stats()[pick];
        let harness = brand::harness(&chosen.target.harness);
        let model = brand::model(&chosen.target.model);
        let top = self.best().unwrap_or(pick);
        let leader = &self.stats()[top];
        let headline = if top == pick || leader.pass_rate() <= chosen.pass_rate() {
            format!(
                "Highest pass rate at {:.1}%, {} total",
                chosen.pass_rate() * 100.0,
                money(chosen.cost())
            )
        } else {
            format!(
                "{} of the best pass rate ({}) for {}",
                percent(chosen.pass_rate(), leader.pass_rate()),
                self.label(top),
                match (chosen.cost(), leader.cost()) {
                    (Some(chosen), Some(cost)) if cost > 0.0 =>
                        format!("{} of the cost", percent(chosen, cost)),
                    _ => "no measured cost".to_owned(),
                }
            )
        };
        let fastest = self
            .stats()
            .iter()
            .all(|stat| stat.p50_seconds >= chosen.p50_seconds);
        vec![
            result(vec![
                strong("Recommendation  "),
                badge(harness),
                strong(harness.label),
                faint("  +  "),
                badge(model),
                strong(chosen.target.model.clone()),
            ]),
            detail(vec![text(headline)]),
            detail(vec![muted(format!(
                "{} p50 latency at {:.1}s, {:.0}% repeated tool calls",
                if fastest { "Fastest" } else { "Competitive" },
                chosen.p50_seconds,
                chosen.repeat_rate * 100.0
            ))]),
        ]
    }

    fn results(&self) -> Vec<Line> {
        let column = |title: &'static str, width: usize, align: Align| Column {
            title,
            width,
            align,
        };
        let columns = [
            column("#", 3, Align::Left),
            column("HARNESS", self.harness_width + 4, Align::Left),
            column("TARGET", self.model_width + 3, Align::Left),
            column("PASS", 7, Align::Right),
            column("FAIL", 5, Align::Right),
            column("ERR", 5, Align::Right),
            column("COST", 8, Align::Right),
            column("¢/PASS", 7, Align::Right),
            column("P50", 6, Align::Right),
        ];
        let rows = self
            .stats()
            .iter()
            .enumerate()
            .map(|(index, stat)| {
                let harness = brand::harness(&stat.target.harness);
                (
                    vec![
                        cell(marker_text(index), self.marker_style(index)),
                        branded(harness, shorten(harness.label, self.harness_width)),
                        branded(
                            brand::model(&stat.target.model),
                            shorten(
                                stat.target.label.as_deref().unwrap_or(&stat.target.model),
                                self.model_width,
                            ),
                        ),
                        cell(format!("{:.1}%", stat.pass_rate() * 100.0), Style::fg(GOOD)),
                        cell(stat.fail.to_string(), Style::fg(BAD)),
                        cell(stat.error.to_string(), Style::fg(WARN)),
                        cell(cost_label(stat), Style::fg(TEXT)),
                        cell(
                            cost_per_pass(stat)
                                .map_or("—".to_owned(), |cost| format!("{:.2}", cost * 100.0)),
                            Style::fg(MUTED),
                        ),
                        cell(format!("{:.1}s", stat.p50_seconds), Style::fg(MUTED)),
                    ],
                    self.picked(index),
                )
            })
            .collect();
        table(&columns, rows, self.width)
            .into_iter()
            .enumerate()
            .filter(|(index, _)| *index != 1)
            .map(|(_, line)| line)
            .collect()
    }

    fn rate_floor(&self) -> f64 {
        let worst = self
            .stats()
            .iter()
            .map(TargetStats::pass_rate)
            .fold(1.0, f64::min);
        ((worst - 0.05) * 10.0 + 1e-9).floor().max(0.0) / 10.0
    }

    fn pass_rates(&self) -> Vec<Line> {
        let floor = self.rate_floor();
        self.stats()
            .iter()
            .enumerate()
            .map(|(index, stat)| {
                self.bar_row(
                    index,
                    (stat.pass_rate() - floor) / (1.0 - floor),
                    format!("{:.1}%", stat.pass_rate() * 100.0),
                )
            })
            .collect()
    }

    fn costs(&self) -> Vec<Line> {
        let most = self
            .stats()
            .iter()
            .filter(|stat| stat.metrics.metrics_complete)
            .filter_map(TargetStats::cost)
            .fold(0.0, f64::max);
        self.stats()
            .iter()
            .enumerate()
            .map(|(index, stat)| {
                self.bar_row(
                    index,
                    ratio(
                        stat.cost()
                            .filter(|_| stat.metrics.metrics_complete)
                            .unwrap_or_default(),
                        most,
                    ),
                    cost_label(stat),
                )
            })
            .collect()
    }

    fn dominator(&self, index: usize) -> Option<usize> {
        let stat = &self.stats()[index];
        let cost = stat.cost()?;
        self.stats()
            .iter()
            .enumerate()
            .find(|(other, rival)| {
                *other != index
                    && rival.cost().is_some_and(|rival_cost| rival_cost <= cost)
                    && rival.pass_rate() >= stat.pass_rate()
                    && (rival.cost().is_some_and(|rival_cost| rival_cost < cost)
                        || rival.pass_rate() > stat.pass_rate())
            })
            .map(|(other, _)| other)
    }

    fn point_label(&self, index: usize) -> Vec<(String, Style)> {
        let target = &self.stats()[index].target;
        let harness = brand::harness(&target.harness);
        let model = brand::model(&target.model);
        [
            (format!("{} ", harness.glyph), Style::fg(harness.color)),
            (harness.label.to_owned(), self.marker_style(index)),
            ("  ".to_owned(), Style::default()),
            (format!("{} ", model.glyph), Style::fg(model.color)),
            (
                shorten(
                    target.label.as_deref().unwrap_or(&target.model),
                    self.model_width,
                ),
                Style::fg(MUTED),
            ),
        ]
        .into_iter()
        .flat_map(|(piece, style)| {
            piece
                .chars()
                .map(move |ch| (ch.to_string(), style))
                .collect::<Vec<_>>()
        })
        .collect()
    }

    fn placed_label(
        &self,
        index: usize,
        column: usize,
        columns: usize,
    ) -> Vec<(usize, String, Style)> {
        let label = self.point_label(index);
        let start = if column + 2 + label.len() <= columns {
            column + 2
        } else {
            column.saturating_sub(label.len() + 1)
        };
        label
            .into_iter()
            .enumerate()
            .map(|(offset, (glyph, style))| (start + offset, glyph, style))
            .filter(|(at, _, _)| *at < columns && *at != column)
            .collect()
    }

    fn scatter(&self) -> Vec<Line> {
        if self
            .stats()
            .iter()
            .any(|stat| stat.cost().is_none() || !stat.metrics.metrics_complete)
        {
            return vec![detail(vec![muted(
                "Cost comparison needs complete model and judge metrics.",
            )])];
        }
        let columns = self.width.saturating_sub(14).min(56);
        let cheapest = self
            .stats()
            .iter()
            .filter_map(TargetStats::cost)
            .fold(f64::MAX, f64::min);
        let priciest = self
            .stats()
            .iter()
            .filter_map(TargetStats::cost)
            .fold(0.0, f64::max);
        let margin = ((priciest - cheapest) * 0.15)
            .max(priciest * 0.05)
            .max(0.01);
        let (low_cost, high_cost) = ((cheapest - margin).max(0.0), priciest + margin);
        let worst = self
            .stats()
            .iter()
            .map(TargetStats::pass_rate)
            .fold(1.0, f64::min);
        let top = self
            .stats()
            .iter()
            .map(TargetStats::pass_rate)
            .fold(0.0, f64::max);
        let (low_rate, high_rate) = ((worst - 0.02).max(0.0), (top + 0.02).min(1.0));
        let points: Vec<(usize, usize, usize)> = self
            .stats()
            .iter()
            .enumerate()
            .map(|(index, stat)| {
                (
                    PLOT_ROWS - 1 - scale(stat.pass_rate(), low_rate, high_rate, PLOT_ROWS),
                    scale(
                        stat.cost().unwrap_or_default(),
                        low_cost,
                        high_cost,
                        columns,
                    ),
                    index,
                )
            })
            .collect();
        let rows = (0..PLOT_ROWS).map(|row| {
            let axis = match row {
                0 => format!("{:.0}%", high_rate * 100.0),
                r if r == PLOT_ROWS - 1 => format!("{:.0}%", low_rate * 100.0),
                _ => String::new(),
            };
            let in_row: Vec<&(usize, usize, usize)> =
                points.iter().filter(|(r, _, _)| *r == row).collect();
            let marks: Vec<(usize, String, Style)> = in_row
                .iter()
                .map(|(_, column, index)| (*column, marker_text(*index), self.marker_style(*index)))
                .chain(
                    in_row
                        .iter()
                        .flat_map(|(_, column, index)| self.placed_label(*index, *column, columns)),
                )
                .collect();
            let cells =
                (0..columns).map(
                    |column| match marks.iter().find(|(at, _, _)| *at == column) {
                        Some((_, glyph, style)) => span(glyph.clone(), *style),
                        None if row % 4 == 0 && column % 4 == 0 => span("·", Style::fg(RULE)),
                        None => gap(1),
                    },
                );
            Line::new(
                [
                    span(pad_left(&axis, 6), Style::fg(FAINT)),
                    span(" │", Style::fg(RULE)),
                ]
                .into_iter()
                .chain(cells)
                .collect(),
            )
        });
        let low = money_axis(low_cost);
        let high = money_axis(high_cost);
        rows.chain([
            Line::new(vec![
                gap(7),
                span(format!("└{}", "─".repeat(columns)), Style::fg(RULE)),
            ]),
            Line::new(vec![
                gap(8),
                faint(pad(&low, columns.saturating_sub(high.len()))),
                faint(high),
            ]),
            Line::new(vec![
                gap(8),
                faint("↑ pass rate   → total cost, model + judge"),
            ]),
            Line::blank(),
        ])
        .chain(self.verdicts())
        .collect()
    }

    fn verdicts(&self) -> Vec<Line> {
        let items: Vec<Vec<Span>> = (0..self.stats().len())
            .map(|index| {
                vec![
                    self.marker(index),
                    match self.dominator(index) {
                        Some(rival) => faint(format!("beaten by {}", marker_text(rival))),
                        None => span("best tradeoff", Style::fg(GOOD)),
                    },
                    gap(4),
                ]
            })
            .collect();
        let limit = self.width.saturating_sub(8);
        items
            .into_iter()
            .fold(Vec::<Vec<Span>>::new(), |rows, item| {
                let item_width = Line::new(item.clone()).width() - 4;
                match rows.split_last() {
                    Some((last, rest)) if Line::new(last.clone()).width() + item_width <= limit => {
                        rest.iter()
                            .cloned()
                            .chain([last.iter().cloned().chain(item).collect()])
                            .collect()
                    }
                    _ => rows.into_iter().chain([item]).collect(),
                }
            })
            .into_iter()
            .map(|spans| Line::new(std::iter::once(gap(8)).chain(spans).collect()))
            .collect()
    }

    fn failures(&self) -> Vec<Line> {
        let most = self
            .stats()
            .iter()
            .map(|stat| stat.fail + stat.error)
            .max()
            .unwrap_or(0)
            .max(1);
        let room = self.track();
        let legend = Line::new(vec![
            gap(self.who_width()),
            span("█", Style::fg(BAD)),
            faint(" Failed attempts  "),
            span("█", Style::fg(WARN)),
            faint(" Errors"),
        ]);
        let rows = self.stats().iter().enumerate().map(|(index, stat)| {
            let cells =
                |count: u32| (f64::from(count) / f64::from(most) * room as f64).round() as usize;
            let failed = cells(stat.fail).min(room);
            let error = cells(stat.error).min(room - failed);
            Line::new(
                self.who(index)
                    .into_iter()
                    .chain([
                        span("█".repeat(failed), Style::fg(BAD)),
                        span("█".repeat(error), Style::fg(WARN)),
                        gap(room - failed - error),
                        span(
                            pad_left(&thousands(u64::from(stat.fail + stat.error)), VALUE),
                            self.marker_style(index),
                        ),
                    ])
                    .collect(),
            )
        });
        std::iter::once(legend).chain(rows).collect()
    }

    fn tools(&self) -> Vec<Line> {
        let most = self
            .stats()
            .iter()
            .map(|stat| stat.tool_calls_per_case)
            .fold(0.0, f64::max);
        let room = self.track().saturating_sub(6);
        self.stats()
            .iter()
            .enumerate()
            .map(|(index, stat)| {
                let repeats = format!("{:>3.0}%↺", stat.repeat_rate * 100.0);
                Line::new(
                    self.who(index)
                        .into_iter()
                        .chain(fill(
                            ratio(stat.tool_calls_per_case, most),
                            room,
                            self.bar_color(index),
                        ))
                        .chain([
                            span(
                                pad_left(&format!("{:.1}/case", stat.tool_calls_per_case), VALUE),
                                self.marker_style(index),
                            ),
                            gap(1),
                            if stat.repeat_rate > 0.15 {
                                warn(repeats)
                            } else {
                                faint(repeats)
                            },
                        ])
                        .collect(),
                )
            })
            .collect()
    }

    fn latency(&self) -> Vec<Line> {
        let room = self.track();
        let slowest = self
            .stats()
            .iter()
            .map(|stat| stat.p95_seconds)
            .fold(0.0, f64::max);
        self.stats()
            .iter()
            .enumerate()
            .map(|(index, stat)| {
                let at = |seconds: f64| scale(seconds, 0.0, slowest, room);
                let (median, tail) = (
                    at(stat.p50_seconds),
                    at(stat.p95_seconds).max(at(stat.p50_seconds)),
                );
                let color = if self.picked(index) { ACCENT } else { TEXT };
                Line::new(
                    self.who(index)
                        .into_iter()
                        .chain([
                            gap(median),
                            span("●", Style::fg(color)),
                            span(
                                "─".repeat(tail.saturating_sub(median + 1)),
                                Style::fg(MUTED),
                            ),
                            span(if tail > median { "┤" } else { "" }, Style::fg(MUTED)),
                            gap(room.saturating_sub(tail + 1)),
                            span(
                                pad_left(
                                    &format!(
                                        "{}/{}",
                                        seconds(stat.p50_seconds),
                                        seconds(stat.p95_seconds)
                                    ),
                                    VALUE,
                                ),
                                self.marker_style(index),
                            ),
                        ])
                        .collect(),
                )
            })
            .collect()
    }

    fn next_steps(&self) -> Vec<Line> {
        let id = &self.summary.run_id;
        let rerun = self
            .summary
            .recommended
            .and_then(|pick| self.stats().get(pick))
            .map(|stat| {
                (
                    format!("lens run --only {}", stat.target.key()),
                    "rerun the pick",
                )
            });
        [
            (format!("lens show {id}"), "print this report again"),
            (format!("lens resume {id}"), "finish an interrupted run"),
        ]
        .into_iter()
        .chain(rerun)
        .map(|(command, why)| {
            let room = self
                .width
                .saturating_sub(4 + why.chars().count() + 2)
                .min(44);
            Line::new(vec![
                gap(2),
                accent("❯ "),
                text(pad(&shorten(&command, room), room + 2)),
                muted(why),
            ])
        })
        .collect()
    }

    fn lines(&self) -> Vec<Line> {
        let width = self.width;
        let title = vec![span("━".repeat(width), Style::fg(ACCENT))];
        let heading = split(
            vec![
                accent("  LENS REPORT  "),
                strong(self.summary.run_id.clone()),
            ],
            joined(vec![
                muted(format!(
                    "{} tasks × {} configs × {} trials",
                    thousands(self.summary.task_count as u64),
                    self.stats().len(),
                    self.summary.trials,
                )),
                muted("local"),
            ]),
            width,
        );
        let floor = self.rate_floor();
        let empty = self.stats().is_empty();
        let sections: Vec<Vec<Line>> = if empty {
            vec![vec![Line::new(vec![
                gap(2),
                muted("No configurations ran."),
            ])]]
        } else {
            vec![
                self.recommendation(),
                section(1, "Results", "¢/PASS = total cost ÷ passed cases", width),
                self.results(),
                section(
                    2,
                    "Pass rate",
                    &if floor > 0.0 {
                        format!("axis starts at {:.0}%", floor * 100.0)
                    } else {
                        String::new()
                    },
                    width,
                ),
                self.pass_rates(),
                section(
                    3,
                    "Total cost",
                    &format!(
                        "model + judge, all {} tasks",
                        thousands(self.summary.task_count as u64)
                    ),
                    width,
                ),
                self.costs(),
                section(4, "Cost vs quality", "up and left is better", width),
                self.scatter(),
                section(5, "Failed attempts", "failed + errored attempts", width),
                self.failures(),
                section(6, "Tool calls", "avg calls per case · ↺ repeated", width),
                self.tools(),
                section(7, "Latency", "● p50   ┤ p95", width),
                self.latency(),
                section(8, "Next", "", width),
                self.next_steps(),
            ]
        };
        [
            vec![
                Line::blank(),
                Line::new(title.clone()),
                heading,
                Line::new(title),
                Line::blank(),
            ],
            sections.concat(),
            vec![
                Line::blank(),
                Line::new(vec![
                    gap(2),
                    faint(format!("Saved to .lens/runs/{}/", self.summary.run_id)),
                ]),
            ],
        ]
        .concat()
        .into_iter()
        .map(|line| line.fit(width))
        .collect()
    }
}

fn fill(fraction: f64, width: usize, color: Rgb) -> Vec<Span> {
    let fraction = if fraction.is_finite() {
        fraction.clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((fraction * width as f64).round() as usize).min(width);
    vec![
        span("█".repeat(filled), Style::fg(color)),
        span("░".repeat(width - filled), Style::fg(RULE)),
    ]
}

fn section(number: usize, title: &str, note: &str, width: usize) -> Vec<Line> {
    vec![
        Line::blank(),
        split(
            vec![accent("▍"), accent(format!("{number:02}  ")), strong(title)],
            vec![faint(note)],
            width,
        ),
        Line::new(vec![span("─".repeat(width), Style::fg(RULE))]),
    ]
}

fn percent(part: f64, whole: f64) -> String {
    format!("{:.0}%", ratio(part, whole) * 100.0)
}

fn ratio(part: f64, whole: f64) -> f64 {
    if whole > 0.0 { part / whole } else { 0.0 }
}

fn scale(value: f64, low: f64, high: f64, steps: usize) -> usize {
    if steps <= 1 || high <= low {
        return 0;
    }
    (((value - low) / (high - low)) * (steps - 1) as f64)
        .round()
        .clamp(0.0, (steps - 1) as f64) as usize
}

fn money(value: Option<f64>) -> String {
    let Some(value) = value else {
        return "unknown".to_owned();
    };
    if value >= 0.01 || value == 0.0 {
        format!("${value:.2}")
    } else {
        format!("${value:.4}")
    }
}

fn cost_label(stat: &TargetStats) -> String {
    if stat.metrics.metrics_complete {
        money(stat.cost())
    } else {
        "incomplete".to_owned()
    }
}

fn money_axis(value: f64) -> String {
    match value {
        v if v >= 10.0 => format!("${v:.0}"),
        v if v >= 1.0 => format!("${v:.1}"),
        v => format!("${v:.3}"),
    }
}

fn seconds(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.0}s")
    } else {
        format!("{value:.1}s")
    }
}

pub fn cost_per_pass(stat: &TargetStats) -> Option<f64> {
    (stat.pass > 0 && stat.metrics.metrics_complete).then_some(())?;
    Some(stat.cost()? / f64::from(stat.pass))
}

pub fn lines(summary: &Summary, width: usize) -> Vec<Line> {
    Report::new(summary, width).lines()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::fixture;
    use crate::local::Target;
    use rstest::rstest;

    fn stat(harness: &str, model: &str, pass: u32, fail: u32, cost: f64) -> TargetStats {
        TargetStats {
            target: Target {
                harness: harness.to_owned(),
                model: model.to_owned(),
                label: None,
            },
            pass,
            fail,
            error: 1,
            model_cost: Some(cost),
            judge_cost: Some(0.0),
            tool_calls_per_case: 2.0,
            repeat_rate: 0.1,
            p50_seconds: 3.0,
            p95_seconds: 7.5,
            metrics: crate::local::Metrics {
                metrics_complete: true,
                ..Default::default()
            },
        }
    }

    fn summary(count: usize, recommended: Option<usize>) -> Summary {
        Summary {
            run_id: "eval_abcd".to_owned(),
            task_count: 40,
            trials: 1,
            stats: (0..count)
                .map(|index| {
                    stat(
                        ["litellm", "openai-agents", "my-loop", "claude-agent-sdk"][index % 4],
                        ["gpt-5-mini", "claude-sonnet-4-5-20250929", "gemini-2.5-pro"][index % 3],
                        30 - index as u32,
                        9 + index as u32,
                        0.05 * (index + 1) as f64,
                    )
                })
                .collect(),
            recommended,
        }
    }

    #[rstest]
    fn demo_recommendation_ratios_come_from_the_data() {
        let summary = fixture::summary();
        let report = Report::new(&summary, 100);
        let lines = report.recommendation();
        assert!(lines[0].plain().contains("Claude Agent SDK"));
        assert!(lines[0].plain().contains("gpt-5-mini"));
        assert_eq!(
            lines[1].plain().trim_start_matches(['⎿', ' ']),
            "97% of the best pass rate (sonnet-5.5) for 51% of the cost"
        );
    }

    #[rstest]
    fn recommending_the_leader_says_so() {
        let lines = Report::new(&summary(2, Some(0)), 100).recommendation();
        assert!(
            lines[1].plain().contains("Highest pass rate at 75.0%"),
            "{}",
            lines[1].plain()
        );
    }

    #[rstest]
    fn no_recommendation_means_no_highlight() {
        let output = lines(&summary(4, None), 100);
        assert!(!output.iter().any(|line| line.fill.is_some()));
        assert!(
            output
                .iter()
                .any(|line| line.plain().contains("No recommendation"))
        );
        assert!(
            !output
                .iter()
                .any(|line| line.plain().contains("lens run --only"))
        );
    }

    #[rstest]
    fn recommended_row_is_highlighted() {
        let output = lines(&summary(4, Some(2)), 100);
        let highlighted: Vec<String> = output
            .iter()
            .filter(|line| line.fill.is_some())
            .map(Line::plain)
            .collect();
        assert_eq!(highlighted.len(), 1);
        assert!(highlighted[0].starts_with("▌ ③"), "{}", highlighted[0]);
    }

    #[rstest]
    #[case::sonnet(0, false)]
    #[case::pick(1, false)]
    #[case::openai_agents(2, true)]
    #[case::langgraph(3, true)]
    fn dominated_configs_are_cheaper_and_better_elsewhere(
        #[case] index: usize,
        #[case] expected: bool,
    ) {
        assert_eq!(
            Report::new(&fixture::summary(), 100)
                .dominator(index)
                .is_some(),
            expected
        );
    }

    #[rstest]
    fn higher_pass_rate_plots_higher() {
        let summary = fixture::summary();
        let plot = Report::new(&summary, 100).scatter();
        let row_of = |index: usize| {
            plot.iter()
                .position(|line| line.plain().contains(&marker_text(index)))
        };
        assert!(row_of(0) < row_of(1));
        assert!(row_of(1) < row_of(3));
        assert!(row_of(3) < row_of(2));
    }

    #[rstest]
    fn single_target_scatter_has_its_point() {
        let plot = Report::new(&summary(1, Some(0)), 100).scatter();
        assert!(plot.iter().any(|line| line.plain().contains("① ≋ LiteLLM")));
    }

    #[rstest]
    fn failure_bar_segments_follow_counts() {
        let summary = fixture::summary();
        let row = &Report::new(&summary, 100).failures()[3];
        let red = row
            .spans
            .iter()
            .find(|piece| piece.style.fg == Some(BAD))
            .expect("failed attempts");
        let yellow = row
            .spans
            .iter()
            .find(|piece| piece.style.fg == Some(WARN))
            .expect("errors");
        assert!(red.text.chars().count() > yellow.text.chars().count());
        assert!(row.plain().trim_end().ends_with("450"));
    }

    #[rstest]
    #[case::paid(70.5, 2775, Some(70.5 / 2775.0))]
    #[case::no_passes(3.0, 0, None)]
    fn cost_per_pass_divides_total_cost_by_passes(
        #[case] cost: f64,
        #[case] pass: u32,
        #[case] expected: Option<f64>,
    ) {
        assert_eq!(
            cost_per_pass(&stat("litellm", "gpt-5-mini", pass, 1, cost)),
            expected
        );
    }
}
