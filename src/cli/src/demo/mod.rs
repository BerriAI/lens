pub mod banner;
pub mod brand;
pub mod fixture;
pub mod progress;
pub mod report;
pub mod style;

use std::io::{self, IsTerminal};
use std::path::Path;
use std::time::Duration;

use style::{
    ACCENT, Line, RULE, Rgb, Span, Style, accent, faint, joined, muted, rule, span, split, strong,
};

use crate::local::view::Live;

#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub animate: bool,
    pub color: bool,
    pub width: usize,
    pub height: usize,
}

impl Options {
    pub fn detect(fast: bool) -> Self {
        let terminal = io::stdout().is_terminal();
        let (rows, columns) = console::Term::stdout().size();
        Self {
            animate: terminal && !fast,
            color: terminal && std::env::var_os("NO_COLOR").is_none(),
            width: usize::from(columns).clamp(80, 120) - 2,
            height: usize::from(rows).max(24),
        }
    }

    pub fn live(&self) -> bool {
        self.color && self.animate
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chrome {
    pub who: String,
    pub path: String,
    pub title: String,
    pub meta: Vec<String>,
    pub hints: Vec<String>,
    pub footer: Vec<String>,
    pub speed: f64,
}

impl Chrome {
    pub fn demo() -> Self {
        Self {
            who: "dev@mac".to_owned(),
            path: "~/support-swarm".to_owned(),
            title: "Run evals".to_owned(),
            meta: vec![fixture::RUN_ID.to_owned(), "local".to_owned()],
            hints: vec!["↑ history".to_owned(), "Lens off".to_owned()],
            footer: vec!["demo".to_owned(), "sample data".to_owned()],
            speed: 140.0,
        }
    }

    pub fn local(run_id: &str) -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        let path = std::env::current_dir()
            .map(|directory| tilde(&directory, &home))
            .unwrap_or_default();
        Self {
            who: std::env::var("USER").unwrap_or_else(|_| "you".to_owned()),
            path,
            title: "Run evals".to_owned(),
            meta: vec![run_id.to_owned(), "local".to_owned()],
            hints: vec!["Ctrl-C to stop".to_owned(), format!("lens resume {run_id}")],
            footer: vec!["results stay on disk".to_owned()],
            speed: 1.0,
        }
    }
}

fn tilde(directory: &Path, home: &str) -> String {
    let shown = directory.display().to_string();
    match shown.strip_prefix(home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => shown,
    }
}

pub fn header(chrome: &Chrome, width: usize) -> Line {
    split(
        vec![
            muted(chrome.who.clone()),
            span("  ", Style::default()),
            muted(chrome.path.clone()),
        ],
        vec![
            accent("LENS"),
            faint(concat!("  v", env!("CARGO_PKG_VERSION"))),
        ],
        width,
    )
}

pub fn input_box(command: &str, width: usize) -> Vec<Line> {
    let inner = width.saturating_sub(2);
    let border = Style::fg(RULE);
    let content = Line::new(vec![accent("❯ "), span(command, Style::fg(style::TEXT))]);
    let padding = inner.saturating_sub(content.width() + 2);
    vec![
        Line::new(vec![span(format!("╭{}╮", "─".repeat(inner)), border)]),
        Line::new(
            [span("│ ", border)]
                .into_iter()
                .chain(content.spans)
                .chain([span(format!("{}│", " ".repeat(padding + 1)), border)])
                .collect(),
        ),
        Line::new(vec![span(format!("╰{}╯", "─".repeat(inner)), border)]),
    ]
}

fn hint_line(chrome: &Chrome, width: usize) -> Line {
    let left = chrome
        .hints
        .iter()
        .map(|hint| faint(hint.clone()))
        .collect();
    split(
        std::iter::once(span("  ", Style::default()))
            .chain(joined(left))
            .collect(),
        joined(
            chrome
                .footer
                .iter()
                .map(|item| faint(item.clone()))
                .collect(),
        ),
        width,
    )
}

fn title_line(chrome: &Chrome, width: usize) -> Line {
    split(
        style::heading("✦", &chrome.title),
        joined(
            chrome
                .meta
                .iter()
                .map(|value| muted(value.clone()))
                .collect(),
        ),
        width,
    )
}

pub fn compose(
    chrome: &Chrome,
    typed: &str,
    body: &[Line],
    options: &Options,
    pinned: bool,
) -> Vec<Line> {
    let width = options.width;
    let top = vec![
        header(chrome, width),
        rule(width),
        Line::blank(),
        title_line(chrome, width),
        Line::blank(),
        Line::new(vec![accent("❯ "), strong(typed)]),
        Line::blank(),
    ];
    let bottom: Vec<Line> = input_box("", width)
        .into_iter()
        .chain(std::iter::once(hint_line(chrome, width)))
        .collect();
    let spacer = if pinned {
        options
            .height
            .saturating_sub(top.len() + body.len() + bottom.len() + 1)
    } else {
        1
    };
    top.into_iter()
        .chain(body.iter().cloned())
        .chain(std::iter::repeat_n(Line::blank(), spacer))
        .chain(bottom)
        .map(|line| line.fit(width))
        .collect()
}

pub fn render(lines: &[Line], options: &Options, separator: &str) -> String {
    lines
        .iter()
        .map(|line| line.render(options.width, options.color))
        .collect::<Vec<_>>()
        .join(separator)
}

pub fn wait(ms: u64, tick: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    let step = 25;
    (0..ms.div_ceil(step)).try_for_each(|_| {
        tick()?;
        std::thread::sleep(Duration::from_millis(step));
        Ok(())
    })
}

const FRAMES: usize = 70;
const FRAME_MS: u64 = 45;

pub fn run(options: Options, tick: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut live = Live::begin(options, fixture::COMMAND, &fixture::meta(), Chrome::demo())?;
    live.type_in(tick)?;
    let total = fixture::event_count();
    let batch = total.div_ceil(FRAMES);
    for frame in 0..FRAMES {
        for index in (frame * batch)..((frame + 1) * batch).min(total) {
            live.record(&fixture::event(index));
        }
        if options.live() {
            live.draw()?;
            wait(FRAME_MS, tick)?;
        }
    }
    live.finish(&fixture::summary())
}

pub fn result(spans: Vec<Span>) -> Line {
    Line::new(
        [span("⏺ ", Style::fg(ACCENT))]
            .into_iter()
            .chain(spans)
            .collect(),
    )
}

pub fn detail(spans: Vec<Span>) -> Line {
    Line::new(
        [span("  ⎿  ", Style::fg(RULE))]
            .into_iter()
            .chain(spans)
            .collect(),
    )
}

pub fn body(spans: Vec<Span>) -> Line {
    Line::indented(2, spans)
}

pub const SPINNER: [&str; 6] = ["·", "✢", "✳", "✶", "✻", "✽"];

pub fn spinner(step: usize, label: &str, elapsed: &str, color: Rgb) -> Line {
    Line::new(vec![
        span(
            format!("{} ", SPINNER[step % SPINNER.len()]),
            Style::fg(color),
        ),
        span(label, Style::fg(color).bold()),
        faint(format!(" ({elapsed} · esc to interrupt)")),
    ])
}

#[cfg(test)]
mod tests {
    use super::tilde;
    use rstest::rstest;
    use std::path::Path;

    #[rstest]
    #[case::inside_home("/Users/dev/support-swarm", "/Users/dev", "~/support-swarm")]
    #[case::outside_home("/srv/app", "/Users/dev", "/srv/app")]
    #[case::no_home("/srv/app", "", "/srv/app")]
    fn path_is_shortened_under_home(
        #[case] directory: &str,
        #[case] home: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(tilde(Path::new(directory), home), expected);
    }
}
