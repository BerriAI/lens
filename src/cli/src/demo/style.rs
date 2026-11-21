use std::fmt::Write;

use unicode_width::UnicodeWidthStr;

use super::brand::Brand;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

pub const ACCENT: Rgb = Rgb(0xD6, 0x8B, 0xFF);
pub const ACCENT_DEEP: Rgb = Rgb(0x6E, 0x22, 0x9A);
pub const ACCENT_TRACK: Rgb = Rgb(0x2C, 0x1B, 0x3C);
pub const HIGHLIGHT: Rgb = Rgb(0x22, 0x17, 0x2E);
pub const RULE: Rgb = Rgb(0x4A, 0x3F, 0x55);
pub const TEXT: Rgb = Rgb(0xEE, 0xEE, 0xEE);
pub const MUTED: Rgb = Rgb(0xA0, 0xA0, 0xA8);
pub const FAINT: Rgb = Rgb(0x6E, 0x6E, 0x78);
pub const GOOD: Rgb = Rgb(0x8F, 0xE3, 0xA0);
pub const BAD: Rgb = Rgb(0xF5, 0x9A, 0x8A);
pub const WARN: Rgb = Rgb(0xF2, 0xCC, 0x6B);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
}

impl Style {
    pub const fn fg(color: Rgb) -> Self {
        Self {
            fg: Some(color),
            bg: None,
            bold: false,
        }
    }

    pub const fn bold(self) -> Self {
        Self { bold: true, ..self }
    }

    pub const fn on(self, color: Rgb) -> Self {
        Self {
            bg: Some(color),
            ..self
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub style: Style,
}

pub fn span(text: impl Into<String>, style: Style) -> Span {
    Span {
        text: text.into(),
        style,
    }
}

pub fn badge(brand: impl Into<Option<Brand>>) -> Span {
    let Some(brand) = brand.into() else {
        return gap(2);
    };
    Span {
        text: format!("{} ", brand.glyph),
        style: Style::fg(brand.color),
    }
}

pub fn text(value: impl Into<String>) -> Span {
    span(value, Style::fg(TEXT))
}

pub fn strong(value: impl Into<String>) -> Span {
    span(value, Style::fg(TEXT).bold())
}

pub fn muted(value: impl Into<String>) -> Span {
    span(value, Style::fg(MUTED))
}

pub fn faint(value: impl Into<String>) -> Span {
    span(value, Style::fg(FAINT))
}

pub fn accent(value: impl Into<String>) -> Span {
    span(value, Style::fg(ACCENT).bold())
}

pub fn good(value: impl Into<String>) -> Span {
    span(value, Style::fg(GOOD))
}

pub fn bad(value: impl Into<String>) -> Span {
    span(value, Style::fg(BAD))
}

pub fn warn(value: impl Into<String>) -> Span {
    span(value, Style::fg(WARN))
}

pub fn gap(width: usize) -> Span {
    span(" ".repeat(width), Style::default())
}

pub const SEPARATOR: &str = "  ·  ";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Line {
    pub spans: Vec<Span>,
    pub fill: Option<Rgb>,
}

impl Line {
    pub fn new(spans: Vec<Span>) -> Self {
        Self { spans, fill: None }
    }

    pub fn blank() -> Self {
        Self::default()
    }

    pub fn indented(indent: usize, spans: Vec<Span>) -> Self {
        Self::new(std::iter::once(gap(indent)).chain(spans).collect())
    }

    pub fn filled(self, color: Rgb) -> Self {
        Self {
            fill: Some(color),
            ..self
        }
    }

    pub fn width(&self) -> usize {
        self.spans.iter().map(|span| span.text.width()).sum()
    }

    pub fn plain(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    pub fn fit(self, width: usize) -> Self {
        if self.width() <= width {
            return self;
        }
        Self {
            spans: self
                .spans
                .into_iter()
                .scan(0usize, |used, piece| {
                    let room = width.saturating_sub(*used);
                    let kept = clip(&piece.text, room);
                    *used += kept.width();
                    Some(Span {
                        text: kept,
                        ..piece
                    })
                })
                .collect(),
            fill: self.fill,
        }
    }

    pub fn render(&self, width: usize, color: bool) -> String {
        let used = self.width();
        if !color {
            return self.plain().trim_end().to_owned();
        }
        let mut output = String::new();
        for span in &self.spans {
            let style = Style {
                bg: span.style.bg.or(self.fill),
                ..span.style
            };
            output.push_str(&sgr(style));
            output.push_str(&span.text);
            output.push_str("\x1b[0m");
        }
        if let Some(fill) = self.fill {
            output.push_str(&sgr(Style::default().on(fill)));
            output.push_str(&" ".repeat(width.saturating_sub(used)));
            output.push_str("\x1b[0m");
        }
        output
    }
}

fn sgr(style: Style) -> String {
    let mut codes = String::from("\x1b[");
    let mut parts = Vec::new();
    if style.bold {
        parts.push("1".to_owned());
    }
    if let Some(Rgb(r, g, b)) = style.fg {
        parts.push(format!("38;2;{r};{g};{b}"));
    }
    if let Some(Rgb(r, g, b)) = style.bg {
        parts.push(format!("48;2;{r};{g};{b}"));
    }
    if parts.is_empty() {
        return String::new();
    }
    let _ = write!(codes, "{}m", parts.join(";"));
    codes
}

pub fn split(left: Vec<Span>, right: Vec<Span>, width: usize) -> Line {
    let left_line = Line::new(left);
    let right_line = Line::new(right);
    let padding = width
        .saturating_sub(left_line.width() + right_line.width())
        .max(1);
    Line::new(
        left_line
            .spans
            .into_iter()
            .chain(std::iter::once(gap(padding)))
            .chain(right_line.spans)
            .collect(),
    )
}

pub fn rule(width: usize) -> Line {
    Line::new(vec![span("─".repeat(width), Style::fg(RULE))])
}

pub fn joined(parts: Vec<Span>) -> Vec<Span> {
    let separator = faint(SEPARATOR);
    parts
        .into_iter()
        .enumerate()
        .flat_map(|(index, part)| {
            (index > 0)
                .then(|| separator.clone())
                .into_iter()
                .chain(std::iter::once(part))
        })
        .collect()
}

pub fn prompt(command: &str) -> Line {
    Line::new(vec![accent("❯ "), strong(command)])
}

pub fn heading(icon: &str, title: &str) -> Vec<Span> {
    vec![accent(icon), gap(2), accent(title)]
}

pub fn bar(fraction: f64, width: usize) -> Vec<Span> {
    let filled = ((fraction.clamp(0.0, 1.0) * width as f64).round() as usize).min(width);
    vec![
        span("█".repeat(filled), Style::fg(ACCENT)),
        span("█".repeat(width - filled), Style::fg(ACCENT_TRACK)),
    ]
}

pub fn clip(value: &str, width: usize) -> String {
    value
        .chars()
        .scan(0usize, |taken, ch| {
            *taken += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            (*taken <= width).then_some(ch)
        })
        .collect()
}

pub fn shorten(value: &str, width: usize) -> String {
    if value.width() <= width {
        return value.to_owned();
    }
    format!("{}…", clip(value, width.saturating_sub(1)))
}

pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let len = digits.len();
    digits
        .chars()
        .enumerate()
        .flat_map(|(index, ch)| {
            (index > 0 && (len - index).is_multiple_of(3))
                .then_some(',')
                .into_iter()
                .chain([ch])
        })
        .collect()
}

pub fn pad(value: &str, width: usize) -> String {
    format!("{value}{}", " ".repeat(width.saturating_sub(value.width())))
}

pub fn pad_left(value: &str, width: usize) -> String {
    format!("{}{value}", " ".repeat(width.saturating_sub(value.width())))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
}

pub struct Column {
    pub title: &'static str,
    pub width: usize,
    pub align: Align,
}

pub struct Cell {
    pub text: String,
    pub style: Style,
    pub brand: Option<Brand>,
}

pub fn cell(text: impl Into<String>, style: Style) -> Cell {
    Cell {
        text: text.into(),
        style,
        brand: None,
    }
}

pub fn branded(brand: impl Into<Option<Brand>>, text: impl Into<String>) -> Cell {
    Cell {
        text: text.into(),
        style: Style::fg(TEXT),
        brand: brand.into(),
    }
}

pub fn table(columns: &[Column], rows: Vec<(Vec<Cell>, bool)>, width: usize) -> Vec<Line> {
    let header = Line::indented(
        2,
        columns
            .iter()
            .map(|column| {
                let title = match column.align {
                    Align::Left => pad(column.title, column.width),
                    Align::Right => pad_left(column.title, column.width),
                };
                span(title, Style::fg(MUTED).bold())
            })
            .collect(),
    );
    let body = rows.into_iter().map(|(cells, selected)| {
        let marker = if selected {
            span("▌ ", Style::fg(ACCENT))
        } else {
            gap(2)
        };
        let spans = std::iter::once(marker)
            .chain(columns.iter().zip(cells).flat_map(|(column, cell)| {
                let icon = cell.brand.map(badge);
                let room = column.width - icon.as_ref().map_or(0, |icon| icon.text.width());
                let value = match column.align {
                    Align::Left => pad(&cell.text, room),
                    Align::Right => pad_left(&cell.text, room),
                };
                let style = if selected {
                    cell.style.bold()
                } else {
                    cell.style
                };
                icon.into_iter().chain([span(value, style)])
            }))
            .collect();
        let line = Line::new(spans);
        if selected {
            line.filled(HIGHLIGHT)
        } else {
            line
        }
    });
    std::iter::once(header)
        .chain(std::iter::once(rule(width)))
        .chain(body)
        .collect()
}
