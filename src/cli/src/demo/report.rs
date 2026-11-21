use crate::demo::brand;
use crate::demo::style::{
    ACCENT, Align, BAD, Column, FAINT, GOOD, Line, MUTED, RULE, Rgb, Span, Style, TEXT, WARN,
    accent, badge, branded, cell, faint, gap, joined, muted, pad, pad_left, shorten, span, split,
    strong, table, text, thousands, warn,
};
use crate::demo::{detail, result};
use crate::local::{Summary, TargetStats};

const VALUE: usize = 9;
const PLOT_ROWS: usize = 19;
const OUTLIER_GAP: f64 = 0.15;
const BAR: &str = "▆";
const FAILURE_KINDS: [(&str, Rgb, &str); 3] = [
    ("▆", BAD, "wrong"),
    ("▄", WARN, "misuse"),
    ("▂", MUTED, "error"),
];
const TRACK: Rgb = Rgb(0x33, 0x2C, 0x3B);
const QUADRANT: Rgb = Rgb(0x31, 0x26, 0x45);
const FRONTIER: Rgb = Rgb(0x9A, 0x5C, 0xC8);
const FIXED_COLUMNS: usize = 2 + 3 + 7 + 5 + 5 + 8 + 7 + 6;

pub fn marker_text(index: usize) -> String {
    (index + 1).to_string()
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
            .map(|stat| stat.target.model.chars().count())
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

    fn rank_width(&self) -> usize {
        marker_text(self.stats().len().saturating_sub(1)).len()
    }

    fn marker(&self, index: usize) -> Span {
        span(
            format!("{} ", pad(&marker_text(index), self.rank_width())),
            self.marker_style(index),
        )
    }

    fn who_width(&self) -> usize {
        2 + self.rank_width() + 1 + 2 + self.harness_width + 2 + 2 + self.model_width + 1
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
                    &shorten(&target.model, self.model_width),
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
                    "No configuration passed any case. Inspect the failures below.",
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
                "Highest pass rate at {:.1}%, ${:.2} total",
                chosen.pass_rate() * 100.0,
                chosen.cost()
            )
        } else {
            format!(
                "{} of the best pass rate ({}) for {}",
                percent(chosen.pass_rate(), leader.pass_rate()),
                self.label(top),
                match leader.cost() {
                    cost if cost > 0.0 => format!("{} of the cost", percent(chosen.cost(), cost)),
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
            column("MODEL", self.model_width + 3, Align::Left),
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
                            shorten(&stat.target.model, self.model_width),
                        ),
                        cell(format!("{:.1}%", stat.pass_rate() * 100.0), Style::fg(GOOD)),
                        cell(stat.fail.to_string(), Style::fg(BAD)),
                        cell(stat.error.to_string(), Style::fg(WARN)),
                        cell(money(stat.cost()), Style::fg(TEXT)),
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
            .map(TargetStats::cost)
            .fold(0.0, f64::max);
        self.stats()
            .iter()
            .enumerate()
            .map(|(index, stat)| self.bar_row(index, ratio(stat.cost(), most), money(stat.cost())))
            .collect()
    }

    fn dominator(&self, index: usize) -> Option<usize> {
        let stat = &self.stats()[index];
        self.stats()
            .iter()
            .enumerate()
            .find(|(other, rival)| {
                *other != index
                    && rival.cost() <= stat.cost()
                    && rival.pass_rate() >= stat.pass_rate()
                    && (rival.cost() < stat.cost() || rival.pass_rate() > stat.pass_rate())
            })
            .map(|(other, _)| other)
    }

    fn frontier(&self) -> Vec<usize> {
        let mut points: Vec<usize> = (0..self.stats().len())
            .filter(|index| self.dominator(*index).is_none())
            .collect();
        points.sort_by(|left, right| {
            self.stats()[*left]
                .cost()
                .total_cmp(&self.stats()[*right].cost())
        });
        points.dedup_by(|later, earlier| {
            let (a, b) = (&self.stats()[*later], &self.stats()[*earlier]);
            a.cost() == b.cost() && a.pass_rate() == b.pass_rate()
        });
        points
    }

    fn harness_short(&self, harness: &str) -> String {
        match harness {
            "claude-agent-sdk" => "Claude SDK".to_owned(),
            other => brand::harness(other).label.to_owned(),
        }
    }

    fn point_label(&self, index: usize) -> String {
        let target = &self.stats()[index].target;
        format!("{} · {}", target.model, self.harness_short(&target.harness))
    }

    fn label_style(&self, index: usize, frontier: &[usize]) -> Style {
        match index {
            pick if self.picked(pick) => Style::fg(ACCENT).bold(),
            best if frontier.contains(&best) => Style::fg(TEXT),
            _ => Style::fg(MUTED),
        }
    }

    fn scatter(&self) -> Vec<Line> {
        let count = self.stats().len();
        let key_width = (0..count)
            .map(|index| self.point_label(index).chars().count())
            .max()
            .unwrap_or(0)
            + self.rank_width()
            + 18;
        let columns = self.width.saturating_sub(9).min(100);
        let costs: Vec<f64> = self.stats().iter().map(TargetStats::cost).collect();
        let axis = CostAxis::new(&costs, columns);
        let rates = RateAxis::new(self.stats().iter().map(TargetStats::pass_rate));
        let frontier = self.frontier();
        let mut grid: Vec<Vec<Option<Span>>> = vec![vec![None; columns]; PLOT_ROWS];
        let mut taken: Vec<Vec<bool>> = vec![vec![false; columns]; PLOT_ROWS];

        let markers: Vec<(usize, usize, usize)> = (0..count)
            .filter(|index| rates.contains(self.stats()[*index].pass_rate()))
            .filter_map(|index| {
                let stat = &self.stats()[index];
                let (row, column) = (rates.row(stat.pass_rate()), axis.column(stat.cost()));
                let width = marker_text(index).chars().count();
                let clear = |at: usize| {
                    at + width <= columns
                        && (at.saturating_sub(1)..(at + width + 1).min(columns))
                            .all(|cell| !taken[row][cell])
                };
                let at = (column..columns)
                    .chain((0..column).rev())
                    .find(|at| clear(*at))?;
                (at..at + width).for_each(|cell| taken[row][cell] = true);
                Some((row, at, index))
            })
            .collect();

        let mut order: Vec<usize> = markers.iter().map(|marker| marker.2).collect();
        order.sort_by_key(|index| (!self.picked(*index), !frontier.contains(index), *index));
        let unlabeled: Vec<usize> = order
            .into_iter()
            .filter(|&index| {
                let Some(&(row, at, _)) = markers.iter().find(|marker| marker.2 == index) else {
                    return false;
                };
                let target = &self.stats()[index].target;
                let lines = [target.model.clone(), self.harness_short(&target.harness)];
                let width = lines
                    .iter()
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0);
                let digits = marker_text(index).chars().count();
                let starts = [Some(at + digits + 1), at.checked_sub(width + 1)];
                let tops = [Some(row), row.checked_sub(1)];
                let spot = tops.into_iter().flatten().find_map(|top| {
                    starts
                        .into_iter()
                        .flatten()
                        .find(|start| {
                            top + lines.len() <= PLOT_ROWS
                                && start + width < columns
                                && (top..top + lines.len()).all(|line_row| {
                                    (start.saturating_sub(1)..(start + width + 1).min(columns))
                                        .all(|cell| !taken[line_row][cell])
                                })
                        })
                        .map(|start| (top, start))
                });
                let Some((top, start)) = spot else {
                    return true;
                };
                let styles = [self.label_style(index, &frontier), Style::fg(FAINT)];
                lines
                    .iter()
                    .zip(styles)
                    .enumerate()
                    .for_each(|(offset, (line, style))| {
                        line.chars().enumerate().for_each(|(column, ch)| {
                            grid[top + offset][start + column] = Some(span(ch.to_string(), style));
                        });
                        (start..start + width).for_each(|cell| taken[top + offset][cell] = true);
                    });
                false
            })
            .collect();

        frontier
            .windows(2)
            .filter_map(|pair| {
                let from = markers.iter().find(|marker| marker.2 == pair[0])?;
                let to = markers.iter().find(|marker| marker.2 == pair[1])?;
                Some((*from, *to))
            })
            .for_each(|((from_row, from_at, _), (to_row, to_at, _))| {
                let steps = to_at
                    .abs_diff(from_at)
                    .max(from_row.abs_diff(to_row))
                    .max(1);
                (1..steps).for_each(|step| {
                    let along = step as f64 / steps as f64;
                    let at =
                        (from_at as f64 + (to_at as f64 - from_at as f64) * along).round() as usize;
                    let row = (from_row as f64 + (to_row as f64 - from_row as f64) * along).round()
                        as usize;
                    if !taken[row][at] && step % 2 == 0 {
                        grid[row][at] = Some(span("·", Style::fg(FRONTIER).bold()));
                    }
                });
            });

        let corner = (rates.row(rates.middle()), axis.column(axis.middle()));
        (0..PLOT_ROWS).for_each(|row| {
            (0..columns).for_each(|at| {
                let divider = match (row == corner.0, at == corner.1) {
                    (_, true) => Some("┊"),
                    (true, false) if at % 2 == 0 => Some("┄"),
                    _ => None,
                };
                let clear =
                    (at.saturating_sub(1)..(at + 2).min(columns)).all(|cell| !taken[row][cell]);
                if let Some(glyph) = divider.filter(|_| clear && grid[row][at].is_none()) {
                    grid[row][at] = Some(span(glyph, Style::fg(RULE)));
                }
            });
        });
        let quadrants = [
            (
                "cheaper + better",
                0..corner.0,
                0..corner.1,
                Style::fg(ACCENT).bold(),
            ),
            (
                "better, pricier",
                0..corner.0,
                corner.1 + 1..columns,
                Style::fg(FAINT),
            ),
            (
                "cheaper, worse",
                corner.0 + 1..PLOT_ROWS,
                0..corner.1,
                Style::fg(FAINT),
            ),
            (
                "worse + pricier",
                corner.0 + 1..PLOT_ROWS,
                corner.1 + 1..columns,
                Style::fg(FAINT),
            ),
        ];
        quadrants
            .into_iter()
            .for_each(|(label, rows, spread, style)| {
                let length = label.chars().count();
                let fits = |row: usize, start: usize| {
                    start + length <= spread.end
                        && (start.saturating_sub(1)..(start + length + 1).min(columns))
                            .all(|at| !taken[row][at] && grid[row][at].is_none())
                };
                let near_edge: Vec<usize> = if rows.start == 0 {
                    rows.clone().collect()
                } else {
                    rows.clone().rev().collect()
                };
                let spot = near_edge.into_iter().find_map(|row| {
                    spread
                        .clone()
                        .skip(1)
                        .find(|start| fits(row, *start))
                        .map(|start| (row, start))
                });
                if let Some((row, start)) = spot {
                    label.chars().enumerate().for_each(|(offset, ch)| {
                        grid[row][start + offset] = Some(span(ch.to_string(), style));
                        taken[row][start + offset] = true;
                    });
                }
            });

        markers.iter().for_each(|&(row, at, index)| {
            let style = match index {
                pick if self.picked(pick) => Style::fg(ACCENT).bold(),
                best if frontier.contains(&best) => Style::fg(TEXT).bold(),
                _ => Style::fg(MUTED),
            };
            grid[row][at] = Some(span(marker_text(index), style));
        });

        let key: Vec<Line> = (0..count)
            .filter(|index| {
                unlabeled.contains(index) || !rates.contains(self.stats()[*index].pass_rate())
            })
            .map(|index| {
                let stat = &self.stats()[index];
                let style = self.label_style(index, &frontier);
                let place = if rates.contains(stat.pass_rate()) {
                    String::new()
                } else {
                    " ▼".to_owned()
                };
                Line::new(vec![
                    span(
                        format!("{} ", pad_left(&marker_text(index), self.rank_width())),
                        style,
                    ),
                    span(
                        pad(&self.point_label(index), key_width - self.rank_width() - 18),
                        style,
                    ),
                    faint(format!(
                        "{:>6}  {:>7}{place}",
                        format!("{:.0}%", stat.pass_rate() * 100.0),
                        money(stat.cost())
                    )),
                ])
            })
            .collect();

        let rows: Vec<Line> = grid
            .into_iter()
            .enumerate()
            .map(|(row, cells)| {
                let label = rates
                    .tick(row)
                    .map_or_else(String::new, |rate| format!("{:.0}%", rate * 100.0));
                let tick = if rates.tick(row).is_some() {
                    " ┤"
                } else {
                    " │"
                };
                let body = cells
                    .into_iter()
                    .enumerate()
                    .scan(0usize, |skip, (column, cell)| {
                        if *skip > 0 {
                            *skip -= 1;
                            return Some(None);
                        }
                        let shaded = row < corner.0 && column < corner.1;
                        let cell = cell.unwrap_or_else(|| gap(1));
                        *skip = cell.text.chars().count().saturating_sub(1);
                        Some(Some(if shaded {
                            Span {
                                style: cell.style.on(QUADRANT),
                                ..cell
                            }
                        } else {
                            cell
                        }))
                    })
                    .flatten();
                Line::new(
                    [
                        span(pad_left(&label, 5), Style::fg(FAINT)),
                        span(tick, Style::fg(RULE)),
                    ]
                    .into_iter()
                    .chain(body)
                    .collect(),
                )
            })
            .collect();

        let ticks = axis.ticks();
        let baseline: String = (0..columns)
            .map(|column| {
                if ticks.iter().any(|(at, _)| *at == column) {
                    '┬'
                } else {
                    '─'
                }
            })
            .collect();
        let tick_line = ticks
            .iter()
            .fold(
                (Vec::<Span>::new(), 0usize),
                |(spans, used), (column, label)| {
                    let start = column.saturating_sub(label.len() / 2).max(used);
                    if start + label.len() > columns + 2 {
                        return (spans, used);
                    }
                    let spans = spans
                        .into_iter()
                        .chain([gap(start - used), faint(label.clone())])
                        .collect();
                    (spans, start + label.len() + 1)
                },
            )
            .0;
        let title = format!(
            "Total cost (USD{})",
            if axis.log { ", log scale" } else { "" }
        );
        let legend = Line::new(
            [
                gap(8),
                span("···", Style::fg(FRONTIER).bold()),
                muted(" Pareto line   "),
                span("  ", Style::default().on(QUADRANT)),
                muted(" cheaper + better"),
            ]
            .into_iter()
            .chain(
                self.stats()
                    .iter()
                    .any(|stat| !rates.contains(stat.pass_rate()))
                    .then(|| muted("   ▼ below the chart")),
            )
            .collect(),
        );
        let below_key: Vec<Line> = if key.is_empty() {
            Vec::new()
        } else {
            std::iter::once(Line::blank())
                .chain(
                    key.into_iter()
                        .map(|line| Line::new(std::iter::once(gap(8)).chain(line.spans).collect())),
                )
                .collect()
        };
        [
            legend,
            Line::blank(),
            Line::new(vec![gap(2), faint("Pass rate")]),
        ]
        .into_iter()
        .chain(rows)
        .chain([
            Line::new(vec![gap(6), span(format!("└{baseline}"), Style::fg(RULE))]),
            Line::new(std::iter::once(gap(7)).chain(tick_line).collect()),
            Line::new(vec![
                gap(7 + columns.saturating_sub(title.len()) / 2),
                faint(title),
            ]),
        ])
        .chain(below_key)
        .collect()
    }

    fn failures(&self) -> Vec<Line> {
        let most = self
            .stats()
            .iter()
            .map(|stat| stat.wrong_answer + stat.tool_misuse + stat.error)
            .max()
            .unwrap_or(0)
            .max(1);
        let column = |title: &str| title.chars().count() + 4;
        let counts = FAILURE_KINDS
            .iter()
            .map(|(_, _, title)| column(title))
            .sum::<usize>();
        let space = (self.track() + VALUE).saturating_sub(counts);
        let room = if space >= 8 { space } else { 0 };
        let header = Line::new(
            std::iter::once(gap(self.who_width() + room))
                .chain(FAILURE_KINDS.iter().flat_map(|(glyph, color, title)| {
                    [
                        gap(column(title) - title.chars().count() - 2),
                        span(*glyph, Style::fg(*color)),
                        faint(format!(" {title}")),
                    ]
                }))
                .collect(),
        );
        let rows = self.stats().iter().enumerate().map(|(index, stat)| {
            let values = [stat.wrong_answer, stat.tool_misuse, stat.error];
            let cells =
                |count: u32| (f64::from(count) / f64::from(most) * room as f64).round() as usize;
            let widths = values.iter().fold(Vec::<usize>::new(), |used, value| {
                let left = room - used.iter().sum::<usize>();
                used.into_iter().chain([cells(*value).min(left)]).collect()
            });
            let bars = FAILURE_KINDS
                .iter()
                .zip(&widths)
                .map(|((glyph, color, _), width)| span(glyph.repeat(*width), Style::fg(*color)));
            let numbers = FAILURE_KINDS
                .iter()
                .zip(values)
                .map(|((_, color, title), value)| {
                    let style = if value == 0 {
                        Style::fg(RULE)
                    } else {
                        Style::fg(*color).bold()
                    };
                    span(pad_left(&thousands(u64::from(value)), column(title)), style)
                });
            Line::new(
                self.who(index)
                    .into_iter()
                    .chain(bars)
                    .chain([gap(room - widths.iter().sum::<usize>())])
                    .chain(numbers)
                    .collect(),
            )
        });
        std::iter::once(header).chain(rows).collect()
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
                    "{} tasks × {} configs",
                    thousands(self.summary.task_count as u64),
                    self.stats().len()
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
                section(
                    5,
                    "Why cases fail",
                    "wrong answer · tool misuse · error",
                    width,
                ),
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
        span(BAR.repeat(filled), Style::fg(color)),
        span(BAR.repeat(width - filled), Style::fg(TRACK)),
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

fn money(value: f64) -> String {
    if value >= 0.01 || value == 0.0 {
        format!("${value:.2}")
    } else {
        format!("${value:.4}")
    }
}

fn seconds(value: f64) -> String {
    if value >= 10.0 {
        format!("{value:.0}s")
    } else {
        format!("{value:.1}s")
    }
}

struct CostAxis {
    low: f64,
    high: f64,
    log: bool,
    columns: usize,
}

impl CostAxis {
    fn new(costs: &[f64], columns: usize) -> Self {
        let cheapest = costs.iter().copied().fold(f64::MAX, f64::min);
        let priciest = costs.iter().copied().fold(0.0, f64::max);
        let log = cheapest > 0.0 && priciest / cheapest >= 4.0;
        let (low, high) = if log {
            (cheapest / 1.4, priciest * 1.4)
        } else {
            let margin = ((priciest - cheapest) * 0.15)
                .max(priciest * 0.05)
                .max(0.01);
            ((cheapest - margin).max(0.0), priciest + margin)
        };
        Self {
            low,
            high,
            log,
            columns,
        }
    }

    fn unit(&self, cost: f64) -> f64 {
        let (value, low, high) = if self.log {
            (cost.max(self.low).ln(), self.low.ln(), self.high.ln())
        } else {
            (cost, self.low, self.high)
        };
        ratio(value - low, high - low).clamp(0.0, 1.0)
    }

    fn column(&self, cost: f64) -> usize {
        let last = self.columns.saturating_sub(2);
        (self.unit(cost) * last as f64).round() as usize
    }

    fn middle(&self) -> f64 {
        if self.log {
            (self.low * self.high).sqrt()
        } else {
            (self.low + self.high) / 2.0
        }
    }

    fn ticks(&self) -> Vec<(usize, String)> {
        let values: Vec<f64> = if self.log {
            (-4..=3)
                .flat_map(|power| [1.0, 2.0, 5.0].map(|step| step * 10f64.powi(power)))
                .filter(|value| (self.low..=self.high).contains(value))
                .collect()
        } else {
            (0..4)
                .map(|step| self.low + (self.high - self.low) * f64::from(step) / 3.0)
                .collect()
        };
        values
            .into_iter()
            .map(|value| (self.column(value), money_tick(value)))
            .collect()
    }
}

struct RateAxis {
    low: f64,
    high: f64,
}

impl RateAxis {
    fn new(rates: impl Iterator<Item = f64> + Clone) -> Self {
        let mut sorted: Vec<f64> = rates.clone().collect();
        sorted.sort_by(f64::total_cmp);
        let worst = sorted
            .windows(2)
            .take_while(|pair| pair[1] - pair[0] > OUTLIER_GAP)
            .last()
            .map_or_else(|| sorted.first().copied().unwrap_or(0.0), |pair| pair[1]);
        let top = rates.fold(0.0, f64::max);
        let high = ((top * 20.0 - 1e-9).ceil() / 20.0).clamp(0.1, 1.0);
        let wanted = (high - (worst - 0.05).max(0.0)).max(0.1);
        let span = ((wanted / 0.15 - 1e-9).ceil() * 0.15).min(high);
        Self {
            low: high - span,
            high,
        }
    }

    fn row(&self, rate: f64) -> usize {
        PLOT_ROWS - 1 - scale(rate, self.low, self.high, PLOT_ROWS)
    }

    fn contains(&self, rate: f64) -> bool {
        rate >= self.low - 1e-9
    }

    fn middle(&self) -> f64 {
        (self.low + self.high) / 2.0
    }

    fn tick(&self, row: usize) -> Option<f64> {
        (row % 6 == 0)
            .then(|| self.high - (self.high - self.low) * row as f64 / (PLOT_ROWS - 1) as f64)
    }
}

fn money_tick(value: f64) -> String {
    match value {
        v if v >= 1.0 => format!("${v:.0}"),
        v if v >= 0.01 => format!("${v:.2}"),
        v => format!("${v:.3}"),
    }
}

fn ranked(summary: &Summary) -> Summary {
    let pick = summary
        .recommended
        .and_then(|index| summary.stats.get(index));
    let mut stats: Vec<TargetStats> = summary.stats.clone();
    stats.sort_by(|left, right| {
        let picked = |stat: &TargetStats| pick.is_some_and(|chosen| chosen == stat);
        picked(right)
            .cmp(&picked(left))
            .then(right.pass_rate().total_cmp(&left.pass_rate()))
            .then(left.cost().total_cmp(&right.cost()))
    });
    Summary {
        recommended: pick.map(|_| 0),
        stats,
        ..summary.clone()
    }
}

pub fn cost_per_pass(stat: &TargetStats) -> Option<f64> {
    (stat.pass > 0).then(|| stat.cost() / f64::from(stat.pass))
}

pub fn lines(summary: &Summary, width: usize) -> Vec<Line> {
    Report::new(&ranked(summary), width).lines()
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
            },
            pass,
            fail,
            wrong_answer: fail / 2,
            tool_misuse: fail - fail / 2,
            error: 1,
            model_cost: cost,
            judge_cost: 0.0,
            tool_calls_per_case: 2.0,
            repeat_rate: 0.1,
            p50_seconds: 3.0,
            p95_seconds: 7.5,
        }
    }

    fn summary(count: usize, recommended: Option<usize>) -> Summary {
        Summary {
            run_id: "eval_abcd".to_owned(),
            task_count: 40,
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
                .any(|line| line.plain().contains("No configuration passed any case"))
        );
        assert!(
            !output
                .iter()
                .any(|line| line.plain().contains("lens run --only"))
        );
    }

    #[rstest]
    fn recommended_row_is_ranked_first_and_highlighted() {
        let output = lines(&summary(4, Some(2)), 100);
        let highlighted: Vec<String> = output
            .iter()
            .filter(|line| line.fill.is_some())
            .map(Line::plain)
            .collect();
        assert_eq!(highlighted.len(), 1);
        assert!(highlighted[0].starts_with("▌ 1 "), "{}", highlighted[0]);
        assert!(highlighted[0].contains("my-loop") || highlighted[0].contains("Custom"));
    }

    fn result_rows(summary: &Summary) -> Vec<String> {
        let output: Vec<String> = lines(summary, 118).iter().map(Line::plain).collect();
        let header = output
            .iter()
            .position(|line| line.trim_start().starts_with("#  HARNESS"))
            .expect("results header");
        output[header + 1..header + 1 + summary.stats.len()].to_vec()
    }

    #[rstest]
    fn results_rank_by_pass_rate_then_cost_after_the_pick() {
        let summary = Summary {
            run_id: "eval_abcd".to_owned(),
            task_count: 20,
            stats: vec![
                stat("litellm", "worst", 5, 15, 0.01),
                stat("litellm", "pricey-best", 20, 0, 0.90),
                stat("litellm", "cheap-best", 20, 0, 0.10),
                stat("litellm", "pick", 18, 2, 0.02),
            ],
            recommended: Some(3),
        };
        let order: Vec<&str> = result_rows(&summary)
            .iter()
            .map(|row| {
                ["pick", "cheap-best", "pricey-best", "worst"]
                    .into_iter()
                    .find(|name| row.contains(name))
                    .expect("known model in row")
            })
            .collect();
        assert_eq!(order, ["pick", "cheap-best", "pricey-best", "worst"]);
        assert!(result_rows(&summary)[0].contains(" 1 "));
        assert!(result_rows(&summary)[3].contains(" 4 "));
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

    fn plot_text(report: &Report) -> Vec<String> {
        report.scatter().iter().map(Line::plain).collect()
    }

    fn plot_markers(text: &[String]) -> Vec<(usize, usize)> {
        let columns = text
            .iter()
            .find(|line| line.trim_start().starts_with('└'))
            .map_or(0, |line| line.trim().chars().count() - 1);
        text.iter()
            .skip_while(|line| !line.contains("Pass rate"))
            .skip(1)
            .take_while(|line| !line.trim_start().starts_with('└'))
            .enumerate()
            .flat_map(|(row, line)| {
                let body: String = line.chars().skip(7).take(columns).collect();
                let padded = format!(" {body} ");
                padded
                    .split(|ch: char| matches!(ch, ' ' | '·' | '┊' | '┄'))
                    .filter(|token| {
                        !token.is_empty() && token.chars().all(|ch| ch.is_ascii_digit())
                    })
                    .map(|token| (row, token.parse::<usize>().expect("digits") - 1))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    fn named_on_chart(text: &[String], model: &str, harness: &str) -> bool {
        text.iter().zip(text.iter().skip(1)).any(|(line, next)| {
            line.match_indices(model).any(|(at, _)| {
                let column = line[..at].chars().count();
                next.chars()
                    .skip(column)
                    .collect::<String>()
                    .starts_with(harness)
            })
        })
    }

    #[rstest]
    fn every_target_is_named_on_the_chart_or_in_the_key(
        #[values(1, 4, 12)] count: usize,
        #[values(78, 100, 118)] width: usize,
    ) {
        let summary = summary(count, Some(0));
        let report = Report::new(&summary, width);
        let text = plot_text(&report);
        (0..count).for_each(|index| {
            let target = &summary.stats[index].target;
            let harness = report.harness_short(&target.harness);
            let in_key = text
                .iter()
                .any(|line| line.contains(&report.point_label(index)));
            assert!(
                in_key || named_on_chart(&text, &target.model, &harness),
                "{} missing\n{}",
                report.point_label(index),
                text.join("\n")
            );
        });
    }

    #[rstest]
    fn spread_out_points_are_labeled_on_the_chart() {
        let summary = fixture::summary();
        let report = Report::new(&summary, 118);
        let text = plot_text(&report);
        let labeled = (0..summary.stats.len())
            .filter(|index| {
                let target = &summary.stats[*index].target;
                named_on_chart(&text, &target.model, &report.harness_short(&target.harness))
            })
            .count();
        assert_eq!(labeled, summary.stats.len(), "{}", text.join("\n"));
    }

    #[rstest]
    fn each_plotted_target_has_exactly_one_marker(
        #[values(4, 12)] count: usize,
        #[values(78, 100, 118)] width: usize,
    ) {
        let summary = summary(count, Some(0));
        let text = plot_text(&Report::new(&summary, width));
        let mut found: Vec<usize> = plot_markers(&text)
            .into_iter()
            .map(|(_, index)| index)
            .collect();
        found.sort_unstable();
        assert_eq!(found, (0..count).collect::<Vec<_>>(), "{}", text.join("\n"));
    }

    #[rstest]
    fn higher_pass_rate_plots_higher() {
        let summary = fixture::summary();
        let text = plot_text(&Report::new(&summary, 100));
        let row = |index: usize| {
            plot_markers(&text)
                .into_iter()
                .find(|(_, found)| *found == index)
                .map(|(row, _)| row)
                .expect("marker")
        };
        let rate = |index: usize| summary.stats[index].pass_rate();
        (0..summary.stats.len()).for_each(|left| {
            (0..summary.stats.len())
                .filter(|right| rate(left) > rate(*right))
                .for_each(|right| assert!(row(left) < row(right), "{left} vs {right}"));
        });
    }

    fn tied() -> Summary {
        Summary {
            run_id: "eval_abcd".to_owned(),
            task_count: 20,
            stats: vec![
                stat("litellm", "claude-sonnet-5-5", 0, 20, 0.0001),
                stat("litellm", "gpt-5.4-mini", 20, 0, 0.0039),
                stat("openai-agents", "gpt-5.4-mini", 20, 0, 0.0040),
            ],
            recommended: Some(1),
        }
    }

    #[rstest]
    fn far_outlier_is_listed_below_instead_of_squashing_the_chart(
        #[values(78, 100, 118)] width: usize,
    ) {
        let summary = tied();
        let report = Report::new(&summary, width);
        let text = plot_text(&report);
        let markers = plot_markers(&text);
        assert!(
            !markers.iter().any(|(_, index)| *index == 0),
            "{}",
            text.join("\n")
        );
        let tied_rows: Vec<usize> = markers
            .iter()
            .filter(|(_, index)| *index != 0)
            .map(|(row, _)| *row)
            .collect();
        assert_eq!(tied_rows.len(), 2, "{}", text.join("\n"));
        assert_eq!(tied_rows[0], tied_rows[1], "{}", text.join("\n"));
        let note = text
            .iter()
            .find(|line| line.contains(&report.point_label(0)) && line.contains('▼'))
            .expect("outlier note");
        assert!(note.contains(&report.point_label(0)), "{note}");
    }

    #[rstest]
    fn chart_marks_the_cheaper_and_better_quadrant(#[values(78, 100, 118)] width: usize) {
        let summary = fixture::summary();
        let text = plot_text(&Report::new(&summary, width));
        let plot: Vec<&String> = text
            .iter()
            .skip_while(|line| !line.contains("Pass rate"))
            .take_while(|line| !line.trim_start().starts_with('└'))
            .collect();
        assert!(plot.iter().any(|line| line.contains("cheaper + better")));
        assert!(plot.iter().any(|line| line.contains('┊')));
    }

    #[rstest]
    fn close_rates_stay_on_the_chart() {
        let axis = RateAxis::new([0.80, 0.90, 1.0].into_iter());
        assert!([0.80, 0.90, 1.0].iter().all(|rate| axis.contains(*rate)));
        assert!(axis.row(1.0) < axis.row(0.90));
        assert!(axis.row(0.90) < axis.row(0.80));
    }

    #[rstest]
    fn frontier_lists_only_configs_nothing_beats() {
        let summary = fixture::summary();
        let report = Report::new(&summary, 118);
        let unbeaten: Vec<usize> = (0..summary.stats.len())
            .filter(|index| report.dominator(*index).is_none())
            .collect();
        let mut frontier = report.frontier();
        frontier.sort_unstable();
        assert_eq!(frontier, unbeaten);
        let costs: Vec<f64> = report
            .frontier()
            .iter()
            .map(|index| summary.stats[*index].cost())
            .collect();
        assert!(costs.windows(2).all(|pair| pair[0] <= pair[1]));
    }

    #[rstest]
    #[case::wide_spread(0.01, 0.50, true)]
    #[case::narrow_spread(0.10, 0.20, false)]
    fn log_scale_only_when_costs_spread_widely(
        #[case] cheap: f64,
        #[case] pricey: f64,
        #[case] log: bool,
    ) {
        let axis = CostAxis::new(&[cheap, pricey], 60);
        assert_eq!(axis.log, log);
        assert!(axis.column(cheap) < axis.column(pricey));
        assert!(axis.ticks().iter().all(|(column, _)| *column < 60));
    }

    #[rstest]
    fn rate_axis_covers_every_rate_and_ticks_land_on_whole_percents() {
        let axis = RateAxis::new([0.85, 0.917, 1.0].into_iter());
        assert!(axis.low <= 0.85 && axis.high >= 1.0);
        assert_eq!(axis.row(1.0), 0);
        assert_eq!(axis.row(axis.low), PLOT_ROWS - 1);
        let ticks: Vec<f64> = (0..PLOT_ROWS).filter_map(|row| axis.tick(row)).collect();
        assert!(ticks.len() >= 3);
        assert!(
            ticks
                .iter()
                .all(|tick| ((tick * 100.0) - (tick * 100.0).round()).abs() < 1e-6),
            "{ticks:?}"
        );
    }

    #[rstest]
    fn failure_legend_fits_with_long_model_names(#[values(78, 100)] width: usize) {
        let section = Report::new(&tied(), width).failures();
        section.iter().for_each(|line| {
            assert!(line.width() <= width, "{}", line.plain());
        });
        assert!(
            section[0].plain().trim_end().ends_with("▂ error"),
            "{}",
            section[0].plain()
        );
    }

    #[rstest]
    fn failure_rows_show_each_kind_with_its_own_glyph_and_count() {
        let summary = fixture::summary();
        let stat = &summary.stats[2];
        let row = &Report::new(&summary, 100).failures()[3];
        let segment = |color: Rgb| {
            row.spans
                .iter()
                .find(|piece| piece.style.fg == Some(color) && !piece.text.trim().is_empty())
                .map(|piece| piece.text.clone())
        };
        let wrong = segment(BAD).expect("wrong answers");
        let misuse = segment(WARN).expect("tool misuse");
        assert!(wrong.chars().all(|ch| ch == '▆'), "{wrong}");
        assert!(misuse.chars().all(|ch| ch == '▄'), "{misuse}");
        assert_eq!(
            wrong.chars().count() > misuse.chars().count(),
            stat.wrong_answer > stat.tool_misuse
        );
        let numbers: Vec<u32> = row
            .plain()
            .split_whitespace()
            .rev()
            .take(3)
            .map(|value| value.replace(',', "").parse().expect("count"))
            .collect();
        assert_eq!(
            numbers,
            vec![stat.error, stat.tool_misuse, stat.wrong_answer]
        );
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
