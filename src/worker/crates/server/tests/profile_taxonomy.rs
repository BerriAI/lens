use chrono::{DateTime, Duration, Utc};
use lens_contract::profiles::{
    ActivationRequest, ClassificationWindow, DiscoveryModelChoice, DiscoveryModelOptions,
    DiscoveryPoint, DiscoveryRequest, DiscoveryRun, ProfileSettings, ProfileSettingsUpdate,
    ResetRequest, TaskDefinition, TaskEdit, Taxonomy, TaxonomyRevisionRequest, TaxonomyStatus,
    TaxonomyTarget,
};
use lens_server::profiles::taxonomy::{
    AgentState, Changes, LEASE, ModelRole, TransitionError, activate, claim, complete, edit_task,
    reset, resume, start_discovery, update_settings,
};
use rstest::{fixture, rstest};

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).expect("time")
}

fn later(minutes: i64) -> DateTime<Utc> {
    now() + Duration::minutes(minutes)
}

fn choice() -> DiscoveryModelChoice {
    DiscoveryModelChoice {
        discovery: "claude-haiku".into(),
        embedding: "qwen-embed".into(),
        classification: "typesafe/jev-latest".into(),
    }
}

fn options() -> DiscoveryModelOptions {
    DiscoveryModelOptions {
        discovery: vec!["claude-haiku".into(), "claude-sonnet".into()],
        embedding: vec!["qwen-embed".into()],
        classification: vec!["typesafe/jev-latest".into()],
    }
}

fn settings(enabled: bool) -> ProfileSettings {
    ProfileSettings {
        agent: "agent".into(),
        enabled,
        models: choice(),
        revision: 4,
        updated_at: None,
    }
}

fn request(max_runs: u32) -> DiscoveryRequest {
    DiscoveryRequest {
        agent: "agent".into(),
        start_ms: 1_000,
        end_ms: 9_000,
        max_runs,
        revision: 0,
    }
}

fn task(task_id: &str) -> TaskDefinition {
    TaskDefinition {
        task_id: task_id.into(),
        name: "Debug failing tests".into(),
        description: String::new(),
        inclusion_criteria: String::new(),
        exclusion_criteria: String::new(),
        required_capabilities: Vec::new(),
        example_trace_ids: vec!["trace".into()],
    }
}

fn applied(state: &AgentState, changes: Changes) -> AgentState {
    AgentState {
        working: changes.working.or_else(|| state.working.clone()),
        active: changes.active.or_else(|| state.active.clone()),
        settings: changes.settings.or_else(|| state.settings.clone()),
    }
}

fn shown(state: &AgentState) -> DiscoveryRequest {
    DiscoveryRequest {
        revision: state.under_review().map_or(0, |working| working.revision),
        ..request(100)
    }
}

fn start(state: &AgentState, job_id: &str) -> Result<AgentState, TransitionError> {
    start_discovery(state, "tax", &shown(state), job_id.into(), now())
        .map(|changes| applied(state, changes))
}

fn finish(state: &AgentState, job_id: &str) -> AgentState {
    let working = state.working.clone().expect("working");
    let claimed = claim(&working, now()).expect("claim");
    let attempt = claimed.job.as_ref().map_or(0, |job| job.attempts);
    let done = complete(
        Some(claimed),
        job_id,
        attempt,
        Ok((vec![task("a")], Vec::new())),
        now(),
    )
    .expect("draft");
    AgentState {
        working: Some(done),
        ..state.clone()
    }
}

fn activation(state: &AgentState, backfill: bool) -> ActivationRequest {
    ActivationRequest {
        agent: "agent".into(),
        revision: state.working.as_ref().map_or(0, |working| working.revision),
        backfill,
    }
}

fn activated(state: &AgentState, backfill: bool) -> AgentState {
    applied(
        state,
        activate(state, &activation(state, backfill), now()).expect("activate"),
    )
}

fn update(
    state: &AgentState,
    enabled: bool,
    models: DiscoveryModelChoice,
) -> ProfileSettingsUpdate {
    ProfileSettingsUpdate {
        agent: "agent".into(),
        revision: state
            .settings
            .as_ref()
            .map_or(0, |settings| settings.revision),
        enabled,
        models,
    }
}

fn revision_of(taxonomy: Option<&Taxonomy>) -> TaxonomyRevisionRequest {
    TaxonomyRevisionRequest {
        agent: "agent".into(),
        revision: taxonomy.map_or(0, |taxonomy| taxonomy.revision),
    }
}

fn rename(state: &AgentState, name: &str) -> TaskEdit {
    TaskEdit {
        agent: "agent".into(),
        revision: state.working.as_ref().map_or(0, |working| working.revision),
        name: name.into(),
        description: String::new(),
        inclusion_criteria: String::new(),
        exclusion_criteria: String::new(),
        required_capabilities: Vec::new(),
    }
}

#[fixture]
fn enabled() -> AgentState {
    AgentState {
        settings: Some(settings(true)),
        ..AgentState::default()
    }
}

#[fixture]
fn drafted(enabled: AgentState) -> AgentState {
    finish(&start(&enabled, "job").expect("start"), "job")
}

#[fixture]
fn active(drafted: AgentState) -> AgentState {
    activated(&drafted, false)
}

#[rstest]
#[case::never_configured(AgentState::default())]
#[case::turned_off(AgentState { settings: Some(settings(false)), ..AgentState::default() })]
fn discovery_is_refused_until_the_agent_opts_in(#[case] state: AgentState) {
    assert_eq!(start(&state, "job"), Err(TransitionError::Disabled));
}

#[rstest]
fn both_language_stages_use_the_one_discovery_model_chosen_at_request_time(enabled: AgentState) {
    let started = start(&enabled, "job").expect("start");
    let models = started.working.expect("working").models;

    assert_eq!(models.distill, "claude-haiku");
    assert_eq!(models.synthesis, models.distill);
    assert_eq!(models.embedding, "qwen-embed");
    assert_eq!(models.decision, "typesafe/jev-latest");
}

#[rstest]
fn a_second_discovery_is_refused_while_one_is_running(enabled: AgentState) {
    let started = start(&enabled, "job").expect("start");

    assert_eq!(start(&started, "job-2"), Err(TransitionError::Running));
}

#[rstest]
#[case::too_small(19)]
#[case::too_large(501)]
fn discovery_size_is_bounded(enabled: AgentState, #[case] max_runs: u32) {
    let result = start_discovery(&enabled, "tax", &request(max_runs), "job".into(), now());

    assert_eq!(result, Err(TransitionError::InvalidSize));
}

#[rstest]
fn activation_classifies_only_runs_that_start_from_then_on(drafted: AgentState) {
    let state = activated(&drafted, false);
    let active = state.active().expect("active");

    assert_eq!(active.status, TaxonomyStatus::Frozen);
    assert_eq!(
        active.windows,
        vec![ClassificationWindow {
            start_ms: now().timestamp_millis(),
            end_ms: None
        }]
    );
    assert_eq!(state.under_review(), None);
}

#[rstest]
fn backfill_adds_only_the_draft_discovery_range(drafted: AgentState) {
    let state = activated(&drafted, true);

    assert_eq!(
        state.active().expect("active").windows,
        vec![
            ClassificationWindow {
                start_ms: 1_000,
                end_ms: Some(9_000)
            },
            ClassificationWindow {
                start_ms: now().timestamp_millis(),
                end_ms: None
            },
        ]
    );
}

#[rstest]
fn a_live_draft_cannot_be_activated_after_discovery_is_turned_off(drafted: AgentState) {
    let off = AgentState {
        settings: Some(settings(false)),
        ..drafted.clone()
    };

    assert_eq!(
        activate(&off, &activation(&off, false), now()),
        Err(TransitionError::Disabled)
    );
}

#[rstest]
fn a_new_discovery_replaces_only_the_draft_the_caller_last_saw(drafted: AgentState) {
    let unseen = start_discovery(&drafted, "tax", &request(100), "job-2".into(), now());
    let seen = start_discovery(&drafted, "tax", &shown(&drafted), "job-2".into(), now());

    assert_eq!(unseen, Err(TransitionError::Stale));
    assert_eq!(
        seen.map(|changes| changes.working.map(|working| working.status)),
        Ok(Some(TaxonomyStatus::Discovering))
    );
}

#[rstest]
fn the_active_taxonomy_keeps_rendering_while_a_new_draft_is_discovered(active: AgentState) {
    let before = active.active().cloned();

    let rediscovering = start(&active, "job-2").expect("rediscover");

    assert_eq!(rediscovering.active().cloned(), before);
    assert_eq!(
        rediscovering.under_review().map(|working| working.status),
        Some(TaxonomyStatus::Discovering)
    );
    assert!(
        rediscovering.under_review().map(|working| working.version)
            > before.map(|taxonomy| taxonomy.version)
    );
}

#[rstest]
fn turning_discovery_off_closes_classification_and_cancels_a_running_job(active: AgentState) {
    let rediscovering = start(&active, "job-2").expect("rediscover");
    let off = applied(
        &rediscovering,
        update_settings(
            &rediscovering,
            &update(&rediscovering, false, choice()),
            &DiscoveryModelOptions::default(),
            later(5),
        )
        .expect("disable"),
    );
    let working = off.working.clone().expect("working");
    let claimed_before = rediscovering
        .working
        .clone()
        .and_then(|working| claim(&working, now()));

    assert!(!off.enabled());
    assert_eq!(
        off.active().expect("active").windows,
        vec![ClassificationWindow {
            start_ms: now().timestamp_millis(),
            end_ms: Some(later(5).timestamp_millis())
        }]
    );
    assert_eq!(working.status, TaxonomyStatus::Failed);
    assert_eq!(
        complete(
            Some(working),
            "job-2",
            claimed_before
                .and_then(|claimed| claimed.job)
                .map_or(1, |job| job.attempts),
            Ok((vec![task("late")], Vec::new())),
            later(6)
        ),
        Err(TransitionError::Stale)
    );
}

#[rstest]
fn turning_discovery_back_on_leaves_classification_paused_until_resumed(active: AgentState) {
    let off = applied(
        &active,
        update_settings(
            &active,
            &update(&active, false, choice()),
            &DiscoveryModelOptions::default(),
            later(5),
        )
        .expect("disable"),
    );
    let on = applied(
        &off,
        update_settings(&off, &update(&off, true, choice()), &options(), later(30))
            .expect("enable"),
    );

    let resumed = applied(
        &on,
        resume(&on, &revision_of(on.active()), later(40)).expect("resume"),
    );

    assert_eq!(on.active(), off.active());
    assert_eq!(
        resumed.active().expect("active").windows,
        vec![
            ClassificationWindow {
                start_ms: now().timestamp_millis(),
                end_ms: Some(later(5).timestamp_millis())
            },
            ClassificationWindow {
                start_ms: later(40).timestamp_millis(),
                end_ms: None
            },
        ]
    );
}

#[rstest]
fn resuming_needs_discovery_on_and_classification_stopped(active: AgentState) {
    let off = AgentState {
        settings: Some(settings(false)),
        ..active.clone()
    };

    assert_eq!(
        resume(&off, &revision_of(off.active()), later(1)),
        Err(TransitionError::Disabled)
    );
    assert_eq!(
        resume(&active, &revision_of(active.active()), later(1)),
        Err(TransitionError::AlreadyClassifying)
    );
}

#[rstest]
#[case::missing_alias(DiscoveryModelChoice { embedding: String::new(), ..choice() }, TransitionError::Unconfigured)]
#[case::unknown_chat_model(DiscoveryModelChoice { discovery: "gone".into(), ..choice() }, TransitionError::UnavailableModel(ModelRole::Discovery))]
#[case::unknown_embedding(DiscoveryModelChoice { embedding: "claude-haiku".into(), ..choice() }, TransitionError::UnavailableModel(ModelRole::Embedding))]
#[case::chat_model_as_classifier(DiscoveryModelChoice { classification: "claude-sonnet".into(), ..choice() }, TransitionError::UnavailableModel(ModelRole::Classification))]
fn opting_in_requires_an_available_model_for_each_role(
    #[case] models: DiscoveryModelChoice,
    #[case] expected: TransitionError,
) {
    let state = AgentState::default();

    assert_eq!(
        update_settings(&state, &update(&state, true, models), &options(), now()),
        Err(expected)
    );
}

#[rstest]
fn turning_discovery_off_works_without_a_model_catalog(enabled: AgentState) {
    let gone = DiscoveryModelChoice {
        discovery: "removed-model".into(),
        ..choice()
    };

    let off = update_settings(
        &enabled,
        &update(&enabled, false, gone),
        &DiscoveryModelOptions::default(),
        now(),
    )
    .expect("disable");

    assert_eq!(
        off.settings
            .map(|settings| (settings.enabled, settings.revision)),
        Some((false, 5))
    );
}

#[rstest]
fn saving_settings_trims_aliases_and_rejects_a_stale_form(enabled: AgentState) {
    let padded = DiscoveryModelChoice {
        discovery: " claude-haiku ".into(),
        ..choice()
    };
    let saved = applied(
        &enabled,
        update_settings(&enabled, &update(&enabled, true, padded), &options(), now())
            .expect("save"),
    );

    let stale = update_settings(&saved, &update(&enabled, true, choice()), &options(), now());

    assert_eq!(
        saved.settings.map(|settings| settings.models),
        Some(choice())
    );
    assert_eq!(stale, Err(TransitionError::Stale));
}

#[rstest]
fn removing_the_active_taxonomy_never_reuses_its_generation(active: AgentState) {
    let request = ResetRequest {
        agent: "agent".into(),
        revision: active.active().map_or(0, |taxonomy| taxonomy.revision),
        target: TaxonomyTarget::Active,
    };
    let removed = applied(&active, reset(&active, &request).expect("reset"));

    let rediscovered = start(&removed, "job-2").expect("rediscover");

    assert_eq!(removed.active(), None);
    assert!(
        rediscovered.working.map(|working| working.version)
            > active.active().map(|taxonomy| taxonomy.version)
    );
}

#[rstest]
fn discarding_a_draft_keeps_the_active_taxonomy(active: AgentState) {
    let drafted = finish(&start(&active, "job-2").expect("rediscover"), "job-2");
    let request = ResetRequest {
        agent: "agent".into(),
        revision: drafted
            .working
            .as_ref()
            .map_or(0, |working| working.revision),
        target: TaxonomyTarget::Draft,
    };

    let discarded = applied(&drafted, reset(&drafted, &request).expect("discard"));

    assert_eq!(discarded.under_review(), None);
    assert_eq!(discarded.active(), active.active());
}

#[rstest]
fn a_stale_browser_edit_cannot_overwrite_a_newer_one(drafted: AgentState) {
    let first = applied(
        &drafted,
        edit_task(&drafted, "a", &rename(&drafted, "Fix flaky tests")).expect("first edit"),
    );

    let stale = edit_task(&first, "a", &rename(&drafted, "Something else"));
    let stale_activation = activate(&first, &activation(&drafted, false), now());

    assert_eq!(stale, Err(TransitionError::Stale));
    assert_eq!(stale_activation, Err(TransitionError::Stale));
    assert_eq!(
        first.working.map(|working| working.tasks[0].name.clone()),
        Some("Fix flaky tests".to_owned())
    );
}

#[rstest]
#[case::blank("  ", Err(TransitionError::InvalidEdit))]
#[case::renamed(" Fix flaky tests ", Ok("Fix flaky tests"))]
fn draft_edits_trim_and_validate_names(
    drafted: AgentState,
    #[case] name: &str,
    #[case] expected: Result<&str, TransitionError>,
) {
    let result = edit_task(&drafted, "a", &rename(&drafted, name)).map(|changes| {
        changes
            .working
            .map(|working| working.tasks[0].name.clone())
            .unwrap_or_default()
    });

    assert_eq!(result.as_deref().map_err(Clone::clone), expected);
}

#[rstest]
fn a_finished_discovery_records_its_range_and_actual_sample(enabled: AgentState) {
    let started = start(&enabled, "job").expect("start");
    let claimed = claim(started.working.as_ref().expect("working"), now()).expect("claim");
    let point = DiscoveryPoint {
        team_id: "team".into(),
        api_key_hash: "key".into(),
        trace_id: "trace".into(),
        task_id: Some("a".into()),
        x: 0.0,
        y: 0.0,
    };

    let done = complete(
        Some(claimed),
        "job",
        1,
        Ok((vec![task("a")], vec![point.clone(), point])),
        now(),
    )
    .expect("draft");

    assert_eq!(
        done.discovery,
        Some(DiscoveryRun {
            start_ms: 1_000,
            end_ms: 9_000,
            requested_runs: 100,
            sampled_runs: 2
        })
    );
}

#[rstest]
fn an_older_attempt_cannot_finish_a_reclaimed_job(enabled: AgentState) {
    let started = start(&enabled, "job").expect("start");
    let first = claim(started.working.as_ref().expect("working"), now()).expect("first claim");
    let second = claim(&first, now() + LEASE + Duration::seconds(1)).expect("reclaim");

    let late = complete(
        Some(second.clone()),
        "job",
        1,
        Ok((vec![task("a")], Vec::new())),
        now(),
    );
    let current = complete(
        Some(second),
        "job",
        2,
        Ok((vec![task("a")], Vec::new())),
        now(),
    );

    assert_eq!(late, Err(TransitionError::Stale));
    assert_eq!(
        current.map(|taxonomy| taxonomy.status),
        Ok(TaxonomyStatus::Draft)
    );
}

#[rstest]
#[case::last_attempt_still_leased(2, Err(TransitionError::Running))]
#[case::last_attempt_lease_expired(3, Ok(()))]
fn the_last_attempt_blocks_a_new_discovery_until_its_lease_ends(
    enabled: AgentState,
    #[case] leases_later: i32,
    #[case] expected: Result<(), TransitionError>,
) {
    let after = |leases: i32| now() + LEASE * leases + Duration::seconds(1);
    let started = start(&enabled, "job").expect("start");
    let first = claim(started.working.as_ref().expect("working"), now()).expect("first");
    let second = claim(&first, after(1)).expect("second");
    let third = claim(&second, after(2)).expect("third");
    let running = AgentState {
        working: Some(third),
        ..enabled
    };

    let restarted = start_discovery(
        &running,
        "tax",
        &shown(&running),
        "job-2".into(),
        after(leases_later),
    );

    assert_eq!(restarted.map(|_| ()), expected);
}

#[rstest]
fn a_held_lease_is_not_reclaimed_and_exhausted_jobs_fail(enabled: AgentState) {
    let started = start(&enabled, "job").expect("start");
    let first = claim(started.working.as_ref().expect("working"), now()).expect("first claim");
    let after = |leases: i32| now() + LEASE * leases + Duration::seconds(1);
    let second = claim(&first, after(1)).expect("second");
    let third = claim(&second, after(2)).expect("third");
    let exhausted = claim(&third, after(3)).expect("exhausted");

    assert_eq!(claim(&first, now()), None);
    assert_eq!(exhausted.status, TaxonomyStatus::Failed);
}
