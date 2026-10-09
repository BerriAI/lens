use super::*;
use litellm_storage_clickhouse::{
    Error,
    evals::ScoringLease,
    state::{Change, Snapshot},
};
use serde_json::{Value, json};

pub struct LiveRun {
    pub database: Database,
    pub store: EvalStore,
    pub run: StoredRun,
    pub now: DateTime<Utc>,
}

#[fixture]
pub async fn live_run(
    #[future(awt)] database: Database,
    request: CreateEvalRun,
    cases: Vec<StoredCase>,
) -> LiveRun {
    let store = EvalStore::new(database.independent());
    let now = Utc::now();
    let run = create(&store, "team", request, cases, now).await;
    LiveRun {
        database,
        store,
        run,
        now,
    }
}

impl LiveRun {
    pub async fn key(&self) -> String {
        self.database
            .store
            .keys("eval-run/", "", 100)
            .await
            .unwrap()[0]
            .clone()
    }

    pub async fn snapshot(&self) -> Snapshot {
        self.database.store.read(&self.key().await).await.unwrap()
    }

    pub async fn scoring(&self) -> (StoredRun, ScoringLease) {
        let run = self
            .store
            .finish(&self.run.team, &self.run.run.id, self.now)
            .await
            .unwrap();
        let lease = self
            .store
            .claim_scoring(&run.team, &run.run.id, self.now)
            .await
            .unwrap()
            .unwrap();
        (run, lease)
    }

    pub async fn replace(&self, value: Value) {
        self.database
            .store
            .commit(vec![Change {
                previous: self.snapshot().await,
                value,
            }])
            .await
            .unwrap();
    }
}

pub fn completion(run: &StoredRun) -> RunCompletion {
    RunCompletion {
        summary: Summary {
            passed: 0,
            total: run.cases.len() as u64,
            pass_rate: 0.0,
            cost_per_case: 0.0,
            scores: BTreeMap::new(),
            errors: run.run.expected_trials,
            baseline_run_id: None,
            baseline_version: None,
            regressions: Vec::new(),
            fixed: Vec::new(),
            gate: GateResult {
                passed: true,
                reasons: Vec::new(),
            },
        },
        trials: run.trials.clone(),
        verdicts: run
            .cases
            .iter()
            .map(|case| (case.id.clone(), false))
            .collect(),
    }
}

pub async fn reject_writes(database: &Database, prefix: &str) {
    database
        .sql(&format!(
            "ALTER TABLE lens_state_blobs ADD CONSTRAINT reject_qualification CHECK NOT startsWith(key, '{prefix}')"
        ))
        .await;
}

pub fn storage_failure(error: &EvalError) -> bool {
    matches!(error, EvalError::Storage(_))
}

#[fixture]
pub async fn paged_runs(#[future(awt)] live_run: LiveRun) -> LiveRun {
    let key = live_run.key().await;
    let prefix = key.rsplit_once('/').unwrap().0;
    let runs = (0..130).map(|index| {
        let id = format!("batch-{index:03}");
        let run = StoredRun {
            request: CreateEvalRun {
                agent: if index == 129 { "needle" } else { "other" }.into(),
                ..live_run.run.request.clone()
            },
            run: lens_contract::eval::EvalRun {
                id: id.clone(),
                ..live_run.run.run.clone()
            },
            ..live_run.run.clone()
        };
        Change {
            previous: Snapshot::empty(format!("{prefix}/{id}")),
            value: serde_json::to_value(run).unwrap(),
        }
    });
    live_run
        .database
        .store
        .commit(
            runs.chain(std::iter::once(Change {
                previous: Snapshot::empty(format!("{prefix}/batch-128-deleted")),
                value: Value::Null,
            }))
            .collect(),
        )
        .await
        .unwrap();
    live_run
}

pub fn run_as_array(run: &StoredRun) -> Value {
    json!([
        run.team,
        run.request,
        run.run,
        run.cases,
        run.trials,
        run.verdicts,
        run.created_at,
        run.scoring_at,
        run.finished_at,
        run.scoring_lease
    ])
}

pub fn invalid_state(error: &EvalError) -> bool {
    matches!(error, EvalError::Storage(Error::InvalidState))
}
