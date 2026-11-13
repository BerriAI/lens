use lens_contract::profiles::{DiscoveryModelOptions, DiscoveryRequest, TaxonomyStatus};
use lens_server::profiles::{
    ProfileStore, RunQuery, taxonomy::{AgentState, Changes, DiscoveryCatalog, TaxonomyStore},
    taxonomy_id,
};
use litellm_lens::task_profiles::seed::seed;
use litellm_traces::query::named::ReadAccessParams;
use rstest::{fixture, rstest};

mod profile_store;
use profile_store::Isolated;

const DAY_MS: i64 = 86_400_000;

struct Demo {
    isolated: Isolated,
    message: String,
}

impl std::ops::Deref for Demo {
    type Target = Isolated;

    fn deref(&self) -> &Isolated {
        &self.isolated
    }
}

fn every_team() -> ReadAccessParams {
    ReadAccessParams {
        all_teams: true,
        user_id: String::new(),
        team_ids: Vec::new(),
    }
}

fn id(agent: &str) -> String {
    taxonomy_id(&every_team(), agent).expect("id")
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[fixture]
async fn demo() -> Demo {
    let isolated = profile_store::isolated().await;
    let message = seed(&isolated.client, &isolated.config, &isolated.store).await.expect("seed");
    Demo { isolated, message }
}

#[rstest]
#[tokio::test]
async fn the_seed_stores_sample_tasks_without_opting_any_agent_in(#[future(awt)] demo: Demo) {
    let (research, _) = demo.profiles.load(&id("lens-demo-research")).await.expect("research");
    let (coding, _) = demo.profiles.load(&id("lens-demo-coding")).await.expect("coding");
    let (support, _) = demo.profiles.load(&id("lens-demo-support")).await.expect("support");

    let reseeded = seed(&demo.client, &demo.config, &demo.store).await.expect("reseed");

    assert!(demo.message.ends_with("no model calls"), "{}", demo.message);
    assert_eq!(reseeded, demo.message);
    assert_eq!(research.active().map(|active| (active.sample, active.windows.len())), Some((true, 0)));
    assert_eq!(research.under_review(), None);
    assert_eq!(coding.active(), None);
    assert_eq!(coding.under_review().map(|draft| (draft.status, draft.sample)), Some((TaxonomyStatus::Draft, true)));
    assert_eq!((support.active(), support.under_review()), (None, None));
    assert_eq!([research.settings, coding.settings, support.settings], [None, None, None]);
}

#[rstest]
#[tokio::test]
async fn the_seed_refuses_to_replace_a_real_taxonomy(#[future(awt)] demo: Demo) {
    let research = id("lens-demo-research");
    let (state, heads) = demo.profiles.load(&research).await.expect("load");
    let real = state.active().cloned().map(|active| lens_contract::profiles::Taxonomy {
        sample: false,
        ..active
    });
    demo.profiles
        .save(&research, heads, Changes { active: real.clone(), ..Changes::default() })
        .await
        .expect("real taxonomy");

    let refused = seed(&demo.client, &demo.config, &demo.store).await;
    let (after, _) = demo.profiles.load(&research).await.expect("load");

    assert_eq!(
        refused.map_err(|error| error.to_string()),
        Err("invalid Lens configuration: A demo agent already has a non-sample task taxonomy; the seed changed nothing".to_owned())
    );
    assert_eq!(after.active().cloned(), real);
}

#[rstest]
#[tokio::test]
async fn seeded_runs_resolve_profiles_and_priced_calls_through_the_store(#[future(awt)] demo: Demo) {
    let (state, _): (AgentState, _) = demo.profiles.load(&id("lens-demo-research")).await.expect("load");
    let active = state.active().cloned().expect("active");
    let runs = demo
        .profiles
        .runs(
            every_team(),
            RunQuery {
                agent: "lens-demo-research".into(),
                start_ms: now_ms() - 14 * DAY_MS,
                end_ms: now_ms() + DAY_MS,
                taxonomy_id: active.taxonomy_id.clone(),
                taxonomy_version: active.version,
                windows: active.windows.clone(),
                limit: 5_001,
            },
        )
        .await
        .expect("runs");
    let calls = demo.profiles.calls(every_team(), runs.clone(), 50_000).await.expect("calls");
    let profiles: std::collections::BTreeSet<&str> = runs.iter().map(|run| run.profile_id.as_str()).collect();
    let priced = calls.values().flatten().flat_map(|call| &call.charges).filter(|charge| charge.spend.is_some()).count();

    assert!(profiles.contains("task:demo-research-summarize"), "{profiles:?}");
    assert!(profiles.contains("recorded:Literature review"), "{profiles:?}");
    assert!(profiles.contains("unknown"), "{profiles:?}");
    assert!(!profiles.contains("pending"), "{profiles:?}");
    assert!(priced > 0);
}

#[rstest]
#[tokio::test]
async fn previews_count_seeded_runs_and_show_their_requests_without_a_gateway(#[future(awt)] demo: Demo) {
    let preview = demo
        .profiles
        .preview(DiscoveryRequest {
            agent: "lens-demo-coding".into(),
            start_ms: now_ms() - 14 * DAY_MS,
            end_ms: now_ms(),
            max_runs: 20,
        })
        .await
        .expect("preview");

    assert!(preview.eligible_runs > 20);
    assert_eq!(preview.sample_runs, 20);
    assert_eq!(preview.previews.len(), 3);
    assert!(preview.previews.first().is_some_and(|run| !run.input_preview.is_empty()));
    assert_eq!(demo.profiles.options().await, DiscoveryModelOptions::default());
    assert_eq!(demo.profiles.configured_models().await, Vec::new());
}
