use std::io::{self, Write};
use std::time::{Duration, Instant};

use super::{Progress, RunMeta, Summary};
use crate::demo::progress::{self, Tally};
use crate::demo::style::Line;
use crate::demo::{Chrome, Options, banner, compose, render, report as sections, wait};

const REDRAW: Duration = Duration::from_millis(50);

pub struct Live {
    pub options: Options,
    chrome: Chrome,
    command: String,
    typed: String,
    revealed: Option<usize>,
    running: bool,
    tally: Tally,
    started: Instant,
    drawn: Option<Instant>,
    spin: usize,
    open: bool,
}

impl Live {
    pub fn start(options: Options, command: &str, meta: &RunMeta) -> io::Result<Self> {
        let mut live = Self::begin(options, command, meta, Chrome::local(&meta.id))?;
        live.typed = command.to_owned();
        live.revealed = None;
        live.running = true;
        live.draw()?;
        Ok(live)
    }

    pub fn begin(
        options: Options,
        command: &str,
        meta: &RunMeta,
        chrome: Chrome,
    ) -> io::Result<Self> {
        let live = Self {
            options,
            chrome,
            command: command.to_owned(),
            typed: String::new(),
            revealed: Some(0),
            running: false,
            tally: Tally::new(meta, 0),
            started: Instant::now(),
            drawn: None,
            spin: 0,
            open: options.live(),
        };
        if live.open {
            write_out("\x1b[2J\x1b[?25l")?;
        }
        Ok(live)
    }

    pub fn type_in(&mut self, tick: &mut dyn FnMut() -> io::Result<()>) -> io::Result<()> {
        if !self.open {
            self.typed = self.command.clone();
            self.revealed = None;
            self.running = true;
            return Ok(());
        }
        self.draw()?;
        wait(350, tick)?;
        let command = self.command.clone();
        for ch in command.chars() {
            self.typed.push(ch);
            self.draw()?;
            wait(22, tick)?;
        }
        wait(250, tick)?;
        let banner_rows = banner::banner(self.options.width).len();
        let intro_rows = self.intro().len();
        self.revealed = Some(banner_rows);
        self.draw()?;
        for shown in banner_rows + 1..=intro_rows {
            self.revealed = Some(shown);
            self.draw()?;
            wait(45, tick)?;
        }
        self.revealed = None;
        self.running = true;
        self.started = Instant::now();
        self.draw()
    }

    pub fn record(&mut self, progress: &Progress) {
        self.tally.record(progress);
    }

    pub fn tally(&self) -> &Tally {
        &self.tally
    }

    pub fn update(&mut self, progress: &Progress) -> io::Result<()> {
        self.record(progress);
        let due = self.drawn.is_none_or(|drawn| drawn.elapsed() >= REDRAW);
        if self.open
            && (due || self.tally.complete() || matches!(progress, Progress::Started { .. }))
        {
            return self.draw();
        }
        Ok(())
    }

    fn clock(&self) -> Duration {
        self.started.elapsed().mul_f64(self.chrome.speed)
    }

    fn intro(&self) -> Vec<Line> {
        banner::banner(self.options.width)
            .into_iter()
            .chain(progress::found(&self.tally))
            .collect()
    }

    fn body(&self) -> Vec<Line> {
        let intro = self.intro();
        let shown = self.revealed.unwrap_or(intro.len()).min(intro.len());
        let frame = if self.running {
            progress::frame(&self.tally, self.clock(), self.spin, self.options.width)
        } else {
            Vec::new()
        };
        intro.into_iter().take(shown).chain(frame).collect()
    }

    pub fn draw(&mut self) -> io::Result<()> {
        if !self.open {
            return Ok(());
        }
        self.spin += 1;
        self.drawn = Some(Instant::now());
        let lines = compose(&self.chrome, &self.typed, &self.body(), &self.options, true);
        let frame: String = lines
            .iter()
            .map(|line| format!("{}\x1b[K", line.render(self.options.width, true)))
            .collect::<Vec<_>>()
            .join("\r\n");
        write_out(&format!("\x1b[H{frame}\x1b[J"))
    }

    pub fn finish(mut self, summary: &Summary) -> io::Result<()> {
        self.typed = self.command.clone();
        self.revealed = None;
        self.running = true;
        let body: Vec<Line> = self
            .body()
            .into_iter()
            .chain(sections::lines(summary, self.options.width))
            .collect();
        let lines = compose(&self.chrome, &self.command, &body, &self.options, false);
        let output = render(&lines, &self.options, "\n");
        if self.open {
            self.open = false;
            return write_out(&format!(
                "\x1b[H\x1b[2J\x1b[3J{}\r\n\x1b[?25h",
                output.replace('\n', "\r\n")
            ));
        }
        write_out(&format!("{output}\n"))
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if self.open {
            let _ = write_out("\x1b[?25h\r\n");
        }
    }
}

fn write_out(text: &str) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    stdout.write_all(text.as_bytes())?;
    stdout.flush()
}

pub fn report(summary: &Summary, width: usize) -> Vec<Line> {
    sections::lines(summary, width)
}

pub fn report_screen(options: &Options, command: &str, summary: &Summary) -> Vec<Line> {
    let chrome = Chrome {
        title: "Eval report".to_owned(),
        hints: vec!["↑ history".to_owned()],
        ..Chrome::local(&summary.run_id)
    };
    compose(
        &chrome,
        command,
        &sections::lines(summary, options.width),
        options,
        false,
    )
}

pub fn print_report(options: Options, command: &str, summary: &Summary) -> io::Result<()> {
    let output = render(&report_screen(&options, command, summary), &options, "\n");
    write_out(&format!("{output}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::fixture;
    use rstest::{fixture as rs_fixture, rstest};

    #[rs_fixture]
    fn quiet() -> Options {
        Options {
            animate: false,
            color: false,
            width: 100,
            height: 40,
        }
    }

    #[rs_fixture]
    fn resumed_live(quiet: Options) -> Live {
        let mut live = Live::start(quiet, fixture::COMMAND, &fixture::meta()).expect("start");
        [
            Progress::Started {
                meta: fixture::meta(),
                resumed: 4,
            },
            fixture::event(0),
            fixture::event(1),
            fixture::event(2),
        ]
        .iter()
        .try_for_each(|event| live.update(event))
        .expect("update");
        live
    }

    #[rstest]
    fn live_counts_events_without_drawing(resumed_live: Live) {
        let live = resumed_live;
        assert_eq!(live.tally().done(), 7);
        assert_eq!(live.tally().resumed, 4);
        assert_eq!(
            live.tally()
                .counts
                .iter()
                .map(|count| count.done)
                .sum::<u64>(),
            3
        );
        assert!(!live.open);
    }

    #[rs_fixture]
    fn completed_live(quiet: Options) -> Live {
        let mut live = Live::start(quiet, fixture::COMMAND, &fixture::meta()).expect("start");
        (0..fixture::event_count()).for_each(|index| live.record(&fixture::event(index)));
        live
    }

    #[rstest]
    fn finished_body_shows_progress_then_the_report(completed_live: Live) {
        let live = completed_live;
        let body: Vec<String> = live.body().iter().map(Line::plain).collect();
        let complete = body
            .iter()
            .position(|line| line.contains("12,000 / 12,000 complete"))
            .expect("progress");
        let found = body
            .iter()
            .position(|line| line.contains("Found 3,000 tasks × 4 configurations"))
            .expect("found");
        assert!(found < complete);
    }
}
