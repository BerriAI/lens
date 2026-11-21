mod banner {
    use lens_cli::demo::{banner::*, style::*};
    use rstest::rstest;

    #[rstest]
    fn logo_spells_lens_with_a_shadow() {
        let rows: Vec<String> = logo().iter().map(Line::plain).collect();
        assert_eq!(rows.len(), 6);
        assert!(rows[0].starts_with("██▄         ██████████▄"));
        let shadow = logo()[5]
            .spans
            .iter()
            .filter(|span| span.style.fg == Some(ACCENT_DEEP))
            .count();
        assert!(shadow > 0);
    }
}

mod style {
    use lens_cli::demo::style::*;
    use rstest::rstest;

    #[rstest]
    fn split_right_aligns_to_width() {
        let line = split(vec![text("left")], vec![text("right")], 20);
        assert_eq!(line.width(), 20);
        assert!(line.plain().ends_with("right"));
    }

    #[rstest]
    fn plain_render_has_no_escape_codes() {
        let line = Line::new(vec![accent("❯ "), strong("/run")]).filled(HIGHLIGHT);
        assert_eq!(line.render(40, false), "❯ /run");
        assert!(line.render(40, true).contains("\x1b[48;2;34;23;46m"));
    }

    #[rstest]
    fn bar_fills_proportionally() {
        let spans = bar(0.25, 40);
        assert_eq!(spans[0].text.chars().count(), 10);
        assert_eq!(spans[1].text.chars().count(), 30);
        assert_eq!(bar(2.0, 8)[1].text, "");
    }

    #[rstest]
    fn table_aligns_columns_and_marks_selection() {
        let columns = [
            Column {
                title: "MODEL",
                width: 8,
                align: Align::Left,
            },
            Column {
                title: "PASS",
                width: 6,
                align: Align::Right,
            },
        ];
        let lines = table(
            &columns,
            vec![
                (
                    vec![cell("a", Style::fg(TEXT)), cell("12", Style::fg(GOOD))],
                    false,
                ),
                (
                    vec![cell("b", Style::fg(TEXT)), cell("9", Style::fg(GOOD))],
                    true,
                ),
            ],
            20,
        );
        assert_eq!(lines[0].plain(), "  MODEL     PASS");
        assert_eq!(lines[2].plain(), "  a           12");
        assert_eq!(lines[3].plain(), "▌ b            9");
        assert_eq!(lines[3].fill, Some(HIGHLIGHT));
    }
}

mod brand {
    use lens_cli::demo::brand::*;
    use rstest::rstest;

    #[rstest]
    #[case::sonnet("sonnet-5.5", "Anthropic")]
    #[case::claude_id("claude-sonnet-5-5", "Anthropic")]
    #[case::gpt("gpt-5-mini", "OpenAI")]
    #[case::reasoning("o3-pro", "OpenAI")]
    #[case::provider_prefix("anthropic/claude-sonnet-4-5", "Anthropic")]
    #[case::openai_prefix("openai/gpt-5-mini", "OpenAI")]
    #[case::gemini("gemini/gemini-2.5-pro", "Google")]
    #[case::unknown("llama-4", "Unknown")]
    fn model_brand_follows_family(#[case] name: &str, #[case] provider: &str) {
        assert_eq!(model(name).label, provider);
    }

    #[rstest]
    #[case::claude("claude-agent-sdk", "Claude Agent SDK", "✻")]
    #[case::openai("openai-agents", "OpenAI Agents", "◎")]
    #[case::langgraph("langgraph", "LangGraph", "⬡")]
    #[case::litellm("litellm", "LiteLLM", "≋")]
    #[case::custom_explicit("custom", "Custom", "◇")]
    #[case::custom("my-harness", "Custom", "◇")]
    fn harness_brand_matches_ui_ids(#[case] id: &str, #[case] label: &str, #[case] glyph: &str) {
        assert_eq!((harness(id).label, harness(id).glyph), (label, glyph));
    }
}

mod progress {
    use lens_cli::local::{AgentResult, Attempt};
    use lens_cli::{
        demo::progress::*,
        local::{Outcome, Progress, RunMeta, Target},
    };
    use rstest::{fixture, rstest};
    use std::time::Duration;

    #[fixture]
    fn meta() -> RunMeta {
        RunMeta {
            id: "eval_test".to_owned(),
            tasks_file: "tasks.csv".to_owned(),
            config_file: "evals.py".to_owned(),
            task_data: None,
            config_hash: None,
            targets: vec![
                Target {
                    harness: "litellm".to_owned(),
                    model: "gpt-5-mini".to_owned(),
                    label: None,
                },
                Target {
                    harness: "openai-agents".to_owned(),
                    model: "gpt-5-mini".to_owned(),
                    label: None,
                },
            ],
            task_count: 3,
            trials: 1,
            concurrency: 4,
        }
    }

    fn finished(target: &Target, outcome: Outcome, cost: f64) -> Progress {
        Progress::Finished(Box::new(Attempt {
            task_id: "t".to_owned(),
            trial: 1,
            target: target.clone(),
            latency_seconds: 1.0,
            result: AgentResult {
                outcome,
                output: String::new(),
                cost: Some(cost),
                judge_cost: Some(0.25),
                tool_calls: Vec::new(),
                metrics: lens_cli::local::Metrics {
                    metrics_complete: true,
                    ..Default::default()
                },
                stop_reason: None,
                check_error: None,
                check_detail: None,
                duration_seconds: None,
                run_id: None,
            },
        }))
    }

    fn played(meta: &RunMeta, resumed: usize) -> Tally {
        let events = [
            Progress::Started {
                meta: meta.clone(),
                resumed,
            },
            finished(&meta.targets[0], Outcome::Pass, 1.0),
            finished(
                &meta.targets[0],
                Outcome::Fail {
                    reason: "wrong".to_owned(),
                },
                1.0,
            ),
            finished(
                &meta.targets[1],
                Outcome::Error {
                    kind: "Timeout".to_owned(),
                    message: "slow".to_owned(),
                },
                0.5,
            ),
        ];
        events.iter().fold(Tally::new(meta, 0), |mut tally, event| {
            tally.record(event);
            tally
        })
    }

    #[rstest]
    fn counters_follow_events(meta: RunMeta) {
        let tally = played(&meta, 0);
        assert_eq!(tally.done(), 3);
        assert_eq!(tally.total(), 6);
        assert_eq!(
            tally.counts[0],
            Count {
                done: 2,
                pass: 1,
                fail: 1,
                error: 0,
                cost: Some(2.5)
            }
        );
        assert_eq!(tally.counts[1].error, 1);
        assert!((tally.cost().unwrap() - 3.25).abs() < 1e-9);
    }

    #[rstest]
    fn frame_shows_real_counts_while_running(meta: RunMeta) {
        let lines = frame(&played(&meta, 0), Duration::from_secs(75), 0, 100);
        assert!(lines[0].plain().contains("Running cases…"));
        assert!(lines[0].plain().contains("1m 15s"));
        assert!(lines[2].plain().ends_with(" 50%"));
        assert!(lines[4].plain().contains("3 / 6 complete"));
        assert!(lines[4].plain().contains("$3.25 so far"));
        assert!(lines[6].plain().contains("LiteLLM"));
        assert!(lines[6].plain().contains("1 pass"));
    }

    #[rstest]
    fn resumed_cases_count_toward_completion(meta: RunMeta) {
        let tally = played(&meta, 3);
        assert!(tally.complete());
        let lines = frame(&tally, Duration::from_secs(5), 0, 100);
        assert_eq!(lines.len(), 5);
        assert!(lines[0].plain().contains("6 cases complete"));
        assert!(
            found(&tally)
                .iter()
                .any(|line| line.plain().contains("3 cases already done"))
        );
    }

    #[rstest]
    #[case::seconds(42, "42s")]
    #[case::minutes(492, "8m 12s")]
    #[case::hours(3_720, "1h 02m")]
    fn elapsed_is_human(#[case] seconds: u64, #[case] expected: &str) {
        assert_eq!(elapsed(Duration::from_secs(seconds)), expected);
    }
}

mod chrome {
    use demo::style::Line;
    use lens_cli::demo::{self, *};
    use rstest::rstest;

    fn options(width: usize) -> Options {
        Options {
            animate: false,
            color: false,
            width,
            height: 30,
        }
    }

    #[rstest]
    fn chrome_never_overflows(#[values(78, 100, 118)] width: usize) {
        let body = banner::banner(width);
        let widest = compose(
            &Chrome::demo(),
            fixture::COMMAND,
            &body,
            &options(width),
            false,
        )
        .iter()
        .map(Line::width)
        .max()
        .unwrap_or(0);
        assert!(widest <= width, "{widest} wide at {width}");
    }

    #[rstest]
    fn input_box_is_closed_on_both_sides() {
        let lines = input_box("/compare eval_7f2a", 40);
        assert_eq!(
            lines.iter().map(Line::width).collect::<Vec<_>>(),
            [40, 40, 40]
        );
        assert!(lines[1].plain().starts_with("│ ❯ /compare") && lines[1].plain().ends_with('│'));
    }

    #[rstest]
    fn pinned_frame_fills_the_screen() {
        let frame = compose(&Chrome::demo(), "", &[], &options(100), true);
        assert_eq!(frame.len(), 29);
        assert!(frame[25].plain().starts_with('╭'));
    }
}

mod fixture {
    use lens_cli::{
        demo::{fixture::*, progress::Tally},
        local::TargetStats,
    };
    use rstest::{fixture, rstest};
    #[fixture]
    fn played_fixture() -> Tally {
        (0..event_count())
            .map(event)
            .fold(Tally::new(&meta(), 0), |mut tally, event| {
                tally.record(&event);
                tally
            })
    }

    #[rstest]
    fn simulated_events_reproduce_the_summary_counts(played_fixture: Tally) {
        let tally = played_fixture;
        let summary = summary();
        assert!(tally.complete());
        assert_eq!(tally.counts[2].pass, u64::from(summary.stats[2].pass));
        assert_eq!(tally.counts[2].fail, u64::from(summary.stats[2].fail));
        assert_eq!(tally.counts[2].error, u64::from(summary.stats[2].error));
        assert!(
            (tally.cost().unwrap()
                - summary
                    .stats
                    .iter()
                    .filter_map(TargetStats::cost)
                    .sum::<f64>())
            .abs()
                < 1e-6
        );
    }
}

mod report {
    use lens_cli::{
        demo::{
            fixture,
            report::{lines, marker_text},
            style::Line,
        },
        local::{Summary, run::new_run_id},
    };
    use rstest::rstest;

    fn report_summary(count: usize) -> Summary {
        let source = fixture::summary();
        Summary {
            run_id: new_run_id(),
            stats: source.stats.iter().cycle().take(count).cloned().collect(),
            ..source
        }
    }
    #[rstest]
    fn report_fits_the_frame(
        #[values(1, 4, 8)] count: usize,
        #[values(78, 100, 118)] width: usize,
    ) {
        let summary = report_summary(count);
        assert_eq!(summary.stats.len(), count);
        let widest = lines(&summary, width)
            .iter()
            .map(Line::width)
            .max()
            .unwrap_or(0);
        assert!(widest <= width, "{widest} > {width}");
    }

    #[rstest]
    #[case::first(0, "①")]
    #[case::eighth(7, "⑧")]
    #[case::beyond_circled(25, "26")]
    fn markers_cover_many_targets(#[case] index: usize, #[case] expected: &str) {
        assert_eq!(marker_text(index), expected);
    }
}
