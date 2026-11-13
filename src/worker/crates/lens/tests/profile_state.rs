use chrono::{DateTime, Utc};
use lens_contract::profiles::{
    ActivationRequest, ClassificationWindow, DiscoveryModelChoice, DiscoveryModelOptions,
    ProfileSettings, ProfileSettingsUpdate, TaskDefinition, Taxonomy, TaxonomyModels,
    TaxonomyRevisionRequest, TaxonomyStatus,
};
use lens_server::profiles::{
    ProfileStoreError,
    taxonomy::{AgentState, Changes, TaxonomyStore, activate, resume, update_settings},
};
use litellm_lens::task_profiles::ProfilesApi;
use rstest::{fixture, rstest};

mod profile_store;
use profile_store::Isolated;

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).expect("time")
}

fn settings(enabled: bool) -> ProfileSettings {
    ProfileSettings {
        agent: "agent".into(),
        enabled,
        models: DiscoveryModelChoice {
            discovery: "chat".into(),
            embedding: "embed".into(),
            classification: "jev".into(),
        },
        revision: 1,
        updated_at: None,
    }
}

fn taxonomy(status: TaxonomyStatus, windows: Vec<ClassificationWindow>) -> Taxonomy {
    Taxonomy {
        taxonomy_id: "tax".into(),
        agent: "agent".into(),
        version: 1,
        revision: 1,
        status,
        sample: false,
        tasks: vec![TaskDefinition {
            task_id: "a".into(),
            name: "Fix bugs".into(),
            description: String::new(),
            inclusion_criteria: String::new(),
            exclusion_criteria: String::new(),
            required_capabilities: Vec::new(),
            example_trace_ids: Vec::new(),
        }],
        points: Vec::new(),
        job: None,
        discovered_at: None,
        frozen_at: None,
        error: None,
        models: TaxonomyModels::default(),
        discovery: None,
        windows,
    }
}

fn disable(state: &AgentState) -> Changes {
    update_settings(
        state,
        &ProfileSettingsUpdate {
            agent: "agent".into(),
            revision: 1,
            enabled: false,
            models: settings(false).models,
        },
        &DiscoveryModelOptions::default(),
        now(),
    )
    .expect("disable")
}

#[fixture]
async fn isolated() -> Isolated {
    profile_store::isolated().await
}

async fn seeded(profiles: &ProfilesApi, working: Option<Taxonomy>, active: Option<Taxonomy>) {
    let (_, heads) = profiles.load("tax").await.expect("load");
    profiles
        .save(
            "tax",
            heads,
            Changes {
                working,
                active,
                settings: Some(settings(true)),
            },
        )
        .await
        .expect("seed");
}

#[rstest]
#[tokio::test]
async fn turning_discovery_off_wins_over_a_concurrent_first_activation(
    #[future(awt)] isolated: Isolated,
) {
    let profiles = &isolated.profiles;
    seeded(
        &profiles,
        Some(taxonomy(TaxonomyStatus::Draft, Vec::new())),
        None,
    )
    .await;
    let (browser, browser_heads) = profiles.load("tax").await.expect("load");
    let (switch, switch_heads) = profiles.load("tax").await.expect("load");
    let request = ActivationRequest {
        agent: "agent".into(),
        revision: 1,
        backfill: false,
    };

    profiles
        .save("tax", switch_heads, disable(&switch))
        .await
        .expect("disable");
    let activation = profiles
        .save(
            "tax",
            browser_heads,
            activate(&browser, &request, now()).expect("activate"),
        )
        .await;

    let (after, _) = profiles.load("tax").await.expect("load");
    assert_eq!(activation, Err(ProfileStoreError::Conflict));
    assert_eq!(after.active(), None);
    assert!(!after.enabled());
}

#[rstest]
#[tokio::test]
async fn turning_discovery_off_wins_over_a_concurrent_resume(#[future(awt)] isolated: Isolated) {
    let profiles = &isolated.profiles;
    let paused = taxonomy(
        TaxonomyStatus::Frozen,
        vec![ClassificationWindow {
            start_ms: 0,
            end_ms: Some(10),
        }],
    );
    seeded(&profiles, None, Some(paused.clone())).await;
    let (browser, browser_heads) = profiles.load("tax").await.expect("load");
    let (switch, switch_heads) = profiles.load("tax").await.expect("load");
    let request = TaxonomyRevisionRequest {
        agent: "agent".into(),
        revision: 1,
    };

    profiles
        .save("tax", switch_heads, disable(&switch))
        .await
        .expect("disable");
    let resumed = profiles
        .save(
            "tax",
            browser_heads,
            resume(&browser, &request, now()).expect("resume"),
        )
        .await;

    let (after, _) = profiles.load("tax").await.expect("load");
    assert_eq!(resumed, Err(ProfileStoreError::Conflict));
    assert_eq!(
        after.active().map(|active| active.windows.clone()),
        Some(paused.windows)
    );
    assert!(!after.enabled());
}
