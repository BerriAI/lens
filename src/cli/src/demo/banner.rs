use crate::demo::style::{
    ACCENT, ACCENT_DEEP, Line, Rgb, Span, Style, faint, gap, muted, span, strong,
};

const GLYPHS: [[&str; 5]; 4] = [
    ["#....", "#....", "#....", "#....", "#####"],
    ["#####", "#....", "####.", "#....", "#####"],
    ["#...#", "##..#", "#.#.#", "#..##", "#...#"],
    [".####", "#....", ".###.", "....#", "####."],
];

fn lit(row: isize, column: isize) -> bool {
    let (Ok(row), Ok(column)) = (usize::try_from(row), usize::try_from(column)) else {
        return false;
    };
    let glyph = column / 6;
    let offset = column % 6;
    row < 5 && glyph < GLYPHS.len() && offset < 5 && GLYPHS[glyph][row].as_bytes()[offset] == b'#'
}

fn pixel(y: isize, x: isize) -> Option<Rgb> {
    if lit(y / 2, x / 2) {
        Some(ACCENT)
    } else if y > 0 && x > 0 && lit((y - 1) / 2, (x - 1) / 2) {
        Some(ACCENT_DEEP)
    } else {
        None
    }
}

fn block(top: Option<Rgb>, bottom: Option<Rgb>) -> Span {
    match (top, bottom) {
        (None, None) => gap(1),
        (Some(top), None) => span("▀", Style::fg(top)),
        (None, Some(bottom)) => span("▄", Style::fg(bottom)),
        (Some(top), Some(bottom)) if top == bottom => span("█", Style::fg(top)),
        (Some(top), Some(bottom)) => span("▀", Style::fg(top).on(bottom)),
    }
}

pub fn logo() -> Vec<Line> {
    let columns = GLYPHS.len() as isize * 12;
    (0..6)
        .map(|row| {
            Line::new(
                (0..columns)
                    .map(|x| block(pixel(row * 2, x), pixel(row * 2 + 1, x)))
                    .collect(),
            )
        })
        .collect()
}

pub fn banner(width: usize) -> Vec<Line> {
    let copy: [Vec<Span>; 6] = [
        vec![],
        vec![strong("Lens CLI")],
        vec![muted("Agent evals from your terminal.")],
        vec![muted("Local by default. Lens is optional.")],
        vec![faint(concat!(
            "v",
            env!("CARGO_PKG_VERSION"),
            " · lens --help"
        ))],
        vec![],
    ];
    let art = logo();
    let side_by_side = art[0].width()
        + 6
        + copy
            .iter()
            .map(|words| Line::new(words.clone()).width())
            .max()
            .unwrap_or(0);
    if side_by_side > width {
        return art
            .into_iter()
            .chain(
                copy.into_iter()
                    .filter(|words| !words.is_empty())
                    .map(Line::new),
            )
            .collect();
    }
    art.into_iter()
        .zip(copy)
        .map(|(art, words)| Line::new(art.spans.into_iter().chain([gap(6)]).chain(words).collect()))
        .collect()
}
