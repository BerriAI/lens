use super::support::*;
use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::Lens,
    worker::{Job, LensSettings, Progress, Review, Step, StepKind},
};
use lens_investigations::*;
use rstest::rstest;
use serde_json::{Value, json};

#[rstest]
fn criteria_hash_matches_retained_python(settings: LensSettings) {
    assert_eq!(
        criteria_key(&settings).unwrap(),
        "e185ed2e7a8aa8d090fbaa7c9ea064c0f40a8ba8e80aed8e8eb7a1329c5c62fd"
    );
}

#[rstest]
#[case::operational(json!({"name":"Renamed","monthly_budget":200,"interval_minutes":30,"enabled":false,"concurrency":2,"lookback_hours":72,"sample_percent":10,"agent_name":"other","execution_ids":["one"]}),false)]
#[case::context(json!({"context":"Inspect unrecovered errors"}),true)]
#[case::model(json!({"model":"other-model"}),true)]
#[case::instruction(json!({"checks":[{"id":"retries","instruction":"Find all retries"}]}),true)]
#[case::trimmed_instruction(json!({"checks":[{"id":"retries","instruction":"\u{1f} Find unrecovered retries \u{1c}"}]}),false)]
#[case::disabled_extra(json!({"checks":[{"id":"retries","instruction":"Find unrecovered retries"},{"id":"other","instruction":"Ignore me","enabled":false}]}),false)]
fn only_analysis_criteria_invalidate_checkpoints(
    settings: LensSettings,
    #[case] update: Value,
    #[case] changed: bool,
) {
    let updated = Value::Object(
        json!(settings)
            .as_object()
            .unwrap()
            .iter()
            .chain(update.as_object().unwrap())
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    );
    assert_eq!(
        criteria_key(&decode(updated)).unwrap() != criteria_key(&settings).unwrap(),
        changed
    );
}

#[rstest]
fn criteria_check_order_and_trim_are_canonical(settings: LensSettings) {
    let first = LensSettings {
        context: "\u{1c} Résumé α \u{1f}".into(),
        checks: decode(
            json!([{"id":"z","instruction":" Zed "},{"id":"a","instruction":" Alpha "}]),
        ),
        ..settings
    };
    let reversed = LensSettings {
        context: "Résumé α".into(),
        checks: first.checks.iter().rev().cloned().collect(),
        ..first.clone()
    };
    assert_eq!(
        criteria_key(&first).unwrap(),
        criteria_key(&reversed).unwrap()
    );
    assert_eq!(
        criteria_key(&first).unwrap(),
        "4b2fab2b75b95edc7d89f6702a1bedaba3e66a15a7fb7b41499e76aa3fc1dae2"
    );
}

#[rstest]
#[case::no_extraction(false)]
#[case::with_extraction(true)]
fn mapping_review_rewrites_only_execution_identities(#[case] extraction: bool) {
    let saved = Review {content_version:"unchanged".into(),extraction:extraction.then(||decode(json!({"reasoning":"evidence","observations":[{"check_id":"one","summary":"summary","evidence":[{"execution_id":"foreign","span_id":"span","quote":"quote","role":"counterexample"}]}]}))),..review(0)};
    let mapped = map_review(&saved, |id| format!("mapped:{id}"));
    assert_eq!(mapped.execution_id, "mapped:run-0");
    assert_eq!(mapped.trace_id, saved.trace_id);
    assert_eq!(mapped.content_version, saved.content_version);
    let recovered = map_review(&mapped, |id| id.strip_prefix("mapped:").unwrap().into());
    assert_eq!(json!(recovered), json!(saved));
    if extraction {
        assert_eq!(
            mapped.extraction.unwrap().observations[0].evidence[0].execution_id,
            "mapped:foreign"
        );
    }
}

#[rstest]
fn checkpoint_summary_keeps_identity_but_hides_extracted_content(job: Job) {
    let saved = Review {
        extraction: Some(Default::default()),
        content_version: "version".into(),
        consolidated: true,
        reused: true,
        partial: true,
        ..review(0)
    };
    let updated = add_review(&job, Some(&saved)).unwrap();
    let summary = &updated.reviews[0];
    assert_eq!(summary.execution_id, "run-0");
    assert!(summary.extraction.is_none());
    assert_eq!(summary.content_version, "");
    assert!(summary.consolidated && summary.reused && summary.partial);
    assert_eq!(updated.reviewed, 1);
    assert_eq!(json!(add_review(&updated, None).unwrap()), json!(updated));
    assert!(saved.extraction.is_some());
    assert_eq!(saved.content_version, "version");
}

#[rstest]
#[case::below_limit(2, 0)]
#[case::at_limit(60, 0)]
#[case::beyond_limit(63, 3)]
fn review_history_retains_newest_while_counting_all(
    job: Job,
    #[case] count: usize,
    #[case] first: usize,
) {
    let grown = (0..count).fold(job, |job, index| {
        add_review(&job, Some(&review(index))).unwrap()
    });
    assert_eq!(grown.reviewed, count as i64);
    assert_eq!(grown.reviews.len(), count.min(MAX_REVIEWS));
    assert_eq!(grown.reviews[0].execution_id, format!("run-{first}"));
    assert_eq!(
        grown.reviews.last().unwrap().execution_id,
        format!("run-{}", count - 1)
    );
}

#[rstest]
fn review_count_overflow_does_not_change_history(job: Job) {
    let saved = Job {
        reviewed: i64::MAX,
        ..job
    };
    assert!(matches!(
        add_review(&saved, Some(&review(0))),
        Err(Error::ReviewCount)
    ));
    assert!(saved.reviews.is_empty());
}

#[rstest]
#[case::before_window(-10,vec!["run-0","run-1","run-2"])]
#[case::old_cursor(0,vec!["run-0","run-1","run-2"])]
#[case::start(7,vec!["run-0","run-1","run-2"])]
#[case::middle(8,vec!["run-1","run-2"])]
#[case::end(10,vec![])]
#[case::future(i64::MAX,vec![])]
fn polling_uses_completion_order_cursor(
    job: Job,
    now: DateTime<Utc>,
    #[case] after: i64,
    #[case] expected: Vec<&str>,
) {
    let saved = Job {
        reviewed: 10,
        reviews: vec![
            review(0),
            Review {
                at: now - TimeDelta::hours(2),
                ..review(1)
            },
            review(2),
        ],
        ..job
    };
    let page = reviews_after(&saved, after);
    assert_eq!(
        page.reviews
            .iter()
            .map(|review| review.execution_id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(page.reviewed, 10);
}

#[rstest]
#[case::short(2, 0)]
#[case::at_limit(200, 0)]
#[case::overflow(205, 5)]
fn step_history_is_bounded(
    job: Job,
    now: DateTime<Utc>,
    #[case] count: usize,
    #[case] first: usize,
) {
    let grown = (0..count).fold(job, |job, index| {
        add_step(
            &job,
            Step {
                at: now,
                kind: StepKind::Stage,
                label: format!("step {index}").try_into().unwrap(),
                model: Default::default(),
                purpose: Default::default(),
                prompt_tokens: 0,
                completion_tokens: 0,
                cost: 0.0,
            },
        )
    });
    assert_eq!(grown.steps.len(), count.min(MAX_STEPS));
    assert_eq!(grown.steps[0].label.as_str(), format!("step {first}"));
    assert_eq!(
        grown.steps.last().unwrap().label.as_str(),
        format!("step {}", count - 1)
    );
}

#[rstest]
#[case::missing(None, false)]
#[case::same(Some("Queued"), false)]
#[case::changed(Some("Reviewing"), true)]
#[case::empty(Some(""), true)]
fn progress_renews_lease_and_only_new_stages_add_steps(
    job: Job,
    now: DateTime<Utc>,
    #[case] stage: Option<&str>,
    #[case] step: bool,
) {
    let update = Progress {
        stage: stage.map(str::to_owned),
        ..Default::default()
    };
    let updated = apply_progress(&job, &update, now).unwrap();
    assert_eq!(updated.stage, stage.unwrap_or("Queued"));
    assert_eq!(updated.steps.len(), usize::from(step));
    assert_eq!(updated.lease_until, Some(now + TimeDelta::minutes(5)));
    if step {
        assert_eq!(updated.steps[0].kind, StepKind::Stage);
        assert_eq!(updated.steps[0].at, now);
        assert_eq!(updated.steps[0].label.as_str(), stage.unwrap());
    }
}

#[rstest]
fn progress_updates_are_independent_and_do_not_erase_parallel_lanes(job: Job, now: DateTime<Utc>) {
    let saved = add_review(
        &Job {
            activities: vec![activity("first"), activity("second")],
            reading: decode(
                json!([{"execution_id":"reading","trace_id":"t","agent":"a","started_at":now}]),
            ),
            ..job
        },
        Some(&review(0)),
    )
    .unwrap();
    let update = Progress {
        activity: Some(lens_contract::worker::Activity {
            operations: decode(json!(["python"])),
            ..activity("first")
        }),
        ..Default::default()
    };
    let updated = apply_progress(&saved, &update, now).unwrap();
    assert_eq!(
        json!(updated.activities),
        json!([update.activity.unwrap(), activity("second")])
    );
    assert_eq!(json!(updated.reviews), json!(saved.reviews));
    assert_eq!(json!(updated.coverage), json!(saved.coverage));
    assert_eq!(json!(updated.reading), json!(saved.reading));
    assert_eq!(updated.reviewed, 1);
    let cleared = apply_progress(
        &updated,
        &Progress {
            reading: Some(vec![]),
            coverage: Some(decode(json!({"screened":8}))),
            review: Some(review(1)),
            ..Default::default()
        },
        now,
    )
    .unwrap();
    assert!(cleared.reading.is_empty());
    assert_eq!(cleared.coverage.screened, 8);
    assert_eq!(cleared.reviewed, 2);
}

#[rstest]
#[case::none(None,false,vec!["first","second"])]
#[case::append(Some("third"),false,vec!["first","second","third"])]
#[case::replace(Some("first"),false,vec!["first","second"])]
#[case::finish(Some("first"),true,vec!["second"])]
#[case::finish_absent(Some("third"),true,vec!["first","second"])]
fn activity_updates_preserve_other_work(
    #[case] id: Option<&str>,
    #[case] finished: bool,
    #[case] expected: Vec<&str>,
) {
    let activities = vec![activity("first"), activity("second")];
    let update = id.map(|id| lens_contract::worker::Activity {
        finished,
        label: "Updated".into(),
        ..activity(id)
    });
    let updated = update_activity(&activities, update.as_ref());
    assert_eq!(
        updated
            .iter()
            .map(|activity| activity.id.as_str())
            .collect::<Vec<_>>(),
        expected
    );
    if id == Some("first") && !finished {
        assert_eq!(updated[0].label, "Updated");
    }
}

#[rstest]
fn summarized_history_hides_content_without_changing_identity(lens: Lens, job: Job) {
    let sample = decode(
        json!({"eligible":4,"selected":1,"executions":[{"id":"run","source":"traces","trace_id":"trace","team_id":"alpha","name":"task","start_time":"start","span_count":3,"metadata":[{"key":"secret","value":"private"}]}]}),
    );
    let saved = Lens {
        jobs: vec![Job {
            sample: Some(sample),
            reviews: vec![review(0)],
            reviewed: 1,
            ..job
        }],
        ..lens
    };
    let summary = summarized(&saved);
    let run = &summary.jobs[0];
    assert!(run.reviews.is_empty());
    assert_eq!(run.reviewed, 1);
    assert!(
        run.sample.as_ref().unwrap().executions[0]
            .metadata
            .is_empty()
    );
    let restored = Job {
        sample: saved.jobs[0].sample.clone(),
        reviews: saved.jobs[0].reviews.clone(),
        ..run.clone()
    };
    assert_eq!(json!(restored), json!(saved.jobs[0]));
}

#[rstest]
fn stage_length_errors_leave_the_original_job_unchanged(job: Job, now: DateTime<Utc>) {
    let progress = Progress {
        stage: Some("x".repeat(201)),
        ..Default::default()
    };
    assert!(matches!(
        apply_progress(&job, &progress, now),
        Err(Error::Conversion(_))
    ));
    assert_eq!(job.stage, "Queued");
    assert!(job.steps.is_empty());
}
