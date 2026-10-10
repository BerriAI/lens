mod identity;
#[path = "sessions/support.rs"]
pub mod support;

use lens_contract::{
    auth::{Identity, Role},
    execution::ExecutionId,
    investigations::{FindingSource, Lens, Scope, Worker},
    worker::{Evidence, EvidenceRole, Job, LensSettings},
};
use lens_investigations::{LensRepository, RepositoryError};
use lens_server::investigations::{InvestigationAccess, InvestigationAccessError};
use litellm_storage_clickhouse::investigations::Investigations;
use rstest::{fixture, rstest};
use serde_json::{Value, json};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use support::{ADMIN, Database, SECRET, Server, database};

struct Access {
    configured: bool,
    worker: bool,
}
impl InvestigationAccess for Access {
    async fn verify_finding(
        &self,
        lens: &Lens,
        sources: &[FindingSource],
    ) -> Result<Option<Vec<Evidence>>, InvestigationAccessError> {
        if sources
            .iter()
            .any(|source| source.quote.as_str() != "Recorded task failure")
        {
            return Ok(None);
        }
        Ok(Some(
            sources
                .iter()
                .map(|source| Evidence {
                    execution_id: ExecutionId {
                        source: "traces".into(),
                        team_id: lens.scope.team_id.clone(),
                        trace_id: source.trace_id.clone(),
                        trace_ref: source.trace_ref.clone(),
                    }
                    .encode(),
                    span_id: source.span_id.clone(),
                    quote: source.quote.clone(),
                    role: EvidenceRole::Support,
                })
                .collect(),
        ))
    }

    fn tracing_enabled(&self) -> bool {
        true
    }
    async fn workers(&self, _: &Scope) -> Result<Vec<Worker>, RepositoryError> {
        Ok(Vec::new())
    }
    async fn validate_model(
        &self,
        _: &LensSettings,
        _: &Identity,
    ) -> Result<(), InvestigationAccessError> {
        if self.configured {
            Ok(())
        } else {
            Err(InvestigationAccessError::ModelUnavailable)
        }
    }
    async fn validate_workers(
        &self,
        _: &LensSettings,
        _: &Scope,
    ) -> Result<(), InvestigationAccessError> {
        if self.worker {
            Ok(())
        } else {
            Err(InvestigationAccessError::WorkerUnavailable)
        }
    }
}

#[fixture]
fn seed() -> Lens {
    serde_json::from_str(include_str!("../../parity/seeds/investigation.json")).unwrap()
}

async fn serve(database: &Database, configured: bool, worker: bool) -> Server {
    database
        .serve_router(false, |authentication| {
            lens_server::sessions::router_with_auth(authentication.clone()).merge(
                lens_server::investigations::router(
                    authentication,
                    Investigations(database.store.clone()),
                    Arc::new(Access { configured, worker }),
                ),
            )
        })
        .await
}

#[fixture]
fn finding_import() -> Value {
    json!({"fingerprint":"a".repeat(64),"agent_name":"test-agent","category":"Reliability",
        "title":"Tool returned an error","description":"The recorded tool could not complete its task",
        "suggestion":"Test a bounded retry","limitation":"Unmeasured proposal","priority":"high",
        "evidence":[{"trace_id":"trace","trace_ref":"B".repeat(64),"span_id":"span","quote":"Recorded task failure"}]})
}

#[rstest]
#[case::admin(Role::ProxyAdmin, false, 200)]
#[case::viewer(Role::ProxyAdminViewer, false, 403)]
#[case::internal(Role::InternalUser, false, 403)]
#[case::forged(Role::ProxyAdmin, true, 422)]
#[tokio::test]
async fn finding_import_requires_admin_and_verified_evidence(
    #[future(awt)] database: Database,
    seed: Lens,
    mut finding_import: Value,
    #[case] role: Role,
    #[case] forged: bool,
    #[case] expected: u16,
) {
    let seed = Lens {
        settings: LensSettings {
            agent_name: "test-agent".into(),
            ..seed.settings.clone()
        },
        ..seed
    };
    let repository = Investigations(database.store.clone());
    repository.create(&seed).await.unwrap();
    let server = serve(&database, true, true).await;
    if forged {
        finding_import["evidence"][0]["quote"] = json!("Forged task failure");
    }
    let token = identity::delegated(Identity {
        user_role: role,
        ..Identity::default()
    });
    let response = server
        .client
        .post(
            server
                .url
                .join(&format!("/lens/{}/findings/import", seed.id))
                .unwrap(),
        )
        .bearer_auth(&token)
        .json(&finding_import)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), expected);
    let saved = repository.get(&seed.id).await.unwrap().unwrap();
    if expected != 200 {
        assert_eq!(json!(saved.findings), json!(seed.findings));
        assert_eq!(saved.version, seed.version);
        return;
    }
    let response: Value = response.json().await.unwrap();
    let finding_id = format!("agent-{}", "a".repeat(64));
    assert_eq!(response, json!({"lens_id":seed.id,"finding_id":finding_id}));
    assert_eq!(saved.findings[0].id, finding_id);
    assert_eq!(saved.findings.len(), seed.findings.len() + 1);
    let replay = server
        .client
        .post(
            server
                .url
                .join(&format!("/lens/{}/findings/import", seed.id))
                .unwrap(),
        )
        .bearer_auth(&token)
        .json(&finding_import)
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 200);
    let after = repository.get(&seed.id).await.unwrap().unwrap();
    assert_eq!(json!(after.findings), json!(saved.findings));
}

#[rstest]
#[tokio::test]
async fn recorded_python_investigations_replay_over_http_and_clickhouse(
    #[future(awt)] database: Database,
    seed: Lens,
) {
    Investigations(database.store.clone())
        .initialize()
        .await
        .unwrap();
    Investigations(database.store.clone())
        .create(&seed)
        .await
        .unwrap();
    let server = serve(&database, false, true).await;
    let fixtures =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../parity/fixtures/investigations");
    let report = lens_parity::replay_fixtures(
        &server.url,
        &fixtures,
        &lens_parity::Tokens::new(ADMIN, SECRET),
    )
    .await
    .unwrap();
    assert!(report.failures.is_empty(), "{:#?}", report.failures);
    assert_eq!(report.passed, 42);
}

#[rstest]
#[case::admin(Role::ProxyAdmin,"GET","/lens",json!(null),200)]
#[case::viewer_read(Role::ProxyAdminViewer,"GET","/lens/parity-lens",json!(null),200)]
#[case::viewer_write(Role::ProxyAdminViewer,"POST","/lens/parity-lens/cancel",json!(null),403)]
#[case::internal_read(Role::InternalUser,"GET","/lens/parity-lens/runs/job",json!(null),403)]
#[case::internal_write(Role::InternalUser,"PATCH","/lens/parity-lens/findings/finding",json!({"status":"resolved"}),403)]
#[case::extra_scope(Role::ProxyAdmin,"POST","/lens",json!({"name":"x","model":"analysis","context":"expected","scope":{"team_id":"other"}}),422)]
#[tokio::test]
async fn authenticated_roles_and_body_scope_cannot_bypass_authorization(
    #[future(awt)] database: Database,
    seed: Lens,
    #[case] role: Role,
    #[case] method: &str,
    #[case] path: &str,
    #[case] body: Value,
    #[case] status: u16,
) {
    Investigations(database.store.clone())
        .create(&seed)
        .await
        .unwrap();
    let server = serve(&database, true, true).await;
    let token = identity::delegated(Identity {
        user_role: role,
        user_id: Some("caller".into()),
        team_id: Some("other-team".into()),
        ..Identity::default()
    });
    let response = server
        .client
        .request(method.parse().unwrap(), server.url.join(path).unwrap())
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    let body: Value = response.json().await.unwrap();
    if status == 403 {
        assert_eq!(
            body["detail"],
            if method == "GET" {
                "Lens requires proxy administrator access"
            } else {
                "Only proxy admins can configure or run Lens"
            }
        );
    }
    assert_eq!(
        Investigations(database.store.clone())
            .get(&seed.id)
            .await
            .unwrap()
            .unwrap()
            .version,
        0
    );
}

#[rstest]
#[case::missing(None)]
#[case::foreign(Some("https://other.test"))]
#[tokio::test]
async fn cookie_mutations_require_lens_origin(
    #[future(awt)] database: Database,
    seed: Lens,
    #[case] origin: Option<&str>,
) {
    Investigations(database.store.clone())
        .create(&seed)
        .await
        .unwrap();
    let server = serve(&database, true, true).await;
    let login = server
        .client
        .post(server.endpoint())
        .json(&json!({"token":ADMIN}))
        .send()
        .await
        .unwrap();
    let cookie = login.headers()["set-cookie"]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    let request = server
        .client
        .post(server.url.join("/lens/parity-lens/cancel").unwrap())
        .header("cookie", cookie);
    let request = if let Some(origin) = origin {
        request.header("origin", origin)
    } else {
        request
    };
    let response = request.send().await.unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":"Lens session requests must come from the Lens origin"})
    );
    assert_eq!(
        Investigations(database.store.clone())
            .get(&seed.id)
            .await
            .unwrap()
            .unwrap()
            .version,
        0
    );
}

#[rstest]
#[case::provider_unavailable(false, true, "Choose a model configured on this LiteLLM instance")]
#[case::worker_unavailable(
    true,
    false,
    "No worker can use this analysis model. Choose a model available to the worker's virtual key, or update its model access and pricing."
)]
#[tokio::test]
async fn rejected_execution_dependencies_do_not_create_lenses(
    #[future(awt)] database: Database,
    #[case] configured: bool,
    #[case] worker: bool,
    #[case] reason: &str,
) {
    let server = serve(&database, configured, worker).await;
    let response = server
        .client
        .post(server.url.join("/lens").unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"name":"Research","model":"analysis","context":"Find failed retries"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"detail":reason})
    );
    assert!(
        Investigations(database.store.clone())
            .lenses(&Scope {
                all_teams: true,
                ..Scope::default()
            })
            .await
            .unwrap()
            .is_empty()
    );
}

#[rstest]
#[tokio::test]
async fn create_run_cancel_and_restart_preserve_frozen_settings_and_public_defaults(
    #[future(awt)] database: Database,
) {
    let server = serve(&database, true, true).await;
    let response=server.client.post(server.url.join("/lens").unwrap()).bearer_auth(ADMIN).json(&json!({"name":"Research","model":"analysis","context":"Find failed retries","enabled":false})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let created: Value = response.json().await.unwrap();
    let lens: Lens = serde_json::from_value(created.clone()).unwrap();
    assert_eq!(
        created["scope"],
        json!({"team_id":"","api_key_hash":"","all_teams":true})
    );
    assert_eq!(lens.jobs.len(), 1);
    assert_eq!(created["jobs"][0]["status"], "queued");
    assert_eq!(created["settings"]["filters"], json!([]));
    assert_eq!(created["settings"]["sample_size"], Value::Null);
    let response = server
        .client
        .post(
            server
                .url
                .join(&format!("/lens/{}/cancel", lens.id))
                .unwrap(),
        )
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let response = server
        .client
        .post(server.url.join(&format!("/lens/{}/runs", lens.id)).unwrap())
        .bearer_auth(ADMIN)
        .json(&json!({"agent_name":"support_agent","lookback_hours":3}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let running: Value = response.json().await.unwrap();
    assert_eq!(running["settings"]["agent_name"], "");
    assert_eq!(
        running["jobs"][0]["settings"]["agent_name"],
        "support_agent"
    );
    assert_eq!(running["jobs"][0]["trigger"], "manual");
    assert_eq!(running["jobs"][0]["status"], "queued");
    assert_eq!(running["jobs"][0]["sample"], Value::Null);
    let response = server
        .client
        .post(
            server
                .url
                .join(&format!("/lens/{}/cancel", lens.id))
                .unwrap(),
        )
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let cancelled: Value = response.json().await.unwrap();
    assert_eq!(cancelled["jobs"][0]["status"], "cancelled");
    drop(server);
    let restarted = serve(&database, true, true).await;
    let response = restarted
        .client
        .get(restarted.url.join(&format!("/lens/{}", lens.id)).unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.json::<Value>().await.unwrap(), cancelled);
}

struct ConcurrentUpdate {
    inner: Investigations,
    conflicting: AtomicBool,
}
impl LensRepository for ConcurrentUpdate {
    async fn lenses(&self, scope: &Scope) -> Result<Vec<Lens>, RepositoryError> {
        self.inner.lenses(scope).await
    }
    async fn get(&self, id: &str) -> Result<Option<Lens>, RepositoryError> {
        self.inner.get(id).await
    }
    async fn create(&self, lens: &Lens) -> Result<Lens, RepositoryError> {
        self.inner.create(lens).await
    }
    async fn replace(&self, expected: &Lens, candidate: &Lens) -> Result<Lens, RepositoryError> {
        if self.conflicting.swap(false, Ordering::SeqCst) {
            let concurrent = Lens {
                spent: 7.5,
                revision: expected.revision + 1,
                ..expected.clone()
            };
            self.inner.replace(expected, &concurrent).await?;
        }
        self.inner.replace(expected, candidate).await
    }
    async fn jobs(&self, id: &str, offset: u64) -> Result<Vec<Job>, RepositoryError> {
        self.inner.jobs(id, offset).await
    }
    async fn job(&self, id: &str, job: &str) -> Result<Option<Job>, RepositoryError> {
        self.inner.job(id, job).await
    }
}

#[rstest]
#[tokio::test]
async fn conflicting_mutations_reload_before_reapplying_domain_transition(
    #[future(awt)] database: Database,
    seed: Lens,
) {
    Investigations(database.store.clone())
        .create(&seed)
        .await
        .unwrap();
    let server = database
        .serve_router(false, |authentication| {
            lens_server::investigations::router(
                authentication,
                ConcurrentUpdate {
                    inner: Investigations(database.store.clone()),
                    conflicting: AtomicBool::new(true),
                },
                Arc::new(Access {
                    configured: true,
                    worker: true,
                }),
            )
        })
        .await;
    let response=server.client.put(server.url.join("/lens/parity-lens").unwrap()).bearer_auth(ADMIN).json(&json!({"name":"New criteria","model":"analysis","context":"different expected behavior"})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    let saved: Lens = response.json().await.unwrap();
    assert_eq!(saved.spent, 7.5);
    assert_eq!(saved.revision, seed.revision + 2);
    assert_eq!(saved.version, seed.version + 2);
    assert!(saved.criteria_updated_at.is_some());
    assert_eq!(saved.settings.name.as_str(), "New criteria");
    assert_eq!(saved.findings.len(), seed.findings.len());
}

#[rstest]
#[tokio::test]
async fn watch_all_enables_paused_lenses_once_without_losing_criteria(
    #[future(awt)] database: Database,
    seed: Lens,
) {
    let seed = Lens {
        settings: LensSettings {
            enabled: false,
            ..seed.settings
        },
        ..seed
    };
    Investigations(database.store.clone())
        .create(&seed)
        .await
        .unwrap();
    let validated_enabled = Arc::new(AtomicBool::new(false));
    let server = database
        .serve_router(false, |authentication| {
            lens_server::investigations::router(
                authentication,
                Investigations(database.store.clone()),
                Arc::new(WatchAccess(validated_enabled.clone())),
            )
        })
        .await;
    let response = server
        .client
        .post(server.url.join("/lens/watch-all").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert!(validated_enabled.load(Ordering::SeqCst));
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"watching":[seed.id],"skipped":[]})
    );
    let updated = Investigations(database.store.clone())
        .get(&seed.id)
        .await
        .unwrap()
        .unwrap();
    assert!(updated.settings.enabled);
    assert_eq!(updated.revision, seed.revision + 1);
    assert_eq!(updated.criteria_updated_at, seed.criteria_updated_at);
    assert_eq!(updated.jobs.len(), seed.jobs.len());
    let response = server
        .client
        .post(server.url.join("/lens/watch-all").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.json::<Value>().await.unwrap(),
        json!({"watching":[],"skipped":[]})
    );
    assert_eq!(
        Investigations(database.store.clone())
            .get(&seed.id)
            .await
            .unwrap()
            .unwrap()
            .version,
        updated.version
    );
}

#[rstest]
#[case::list("/lens/", "/lens")]
#[case::read("/lens/parity-lens/", "/lens/parity-lens")]
#[case::runs("/lens/parity-lens/runs/?offset=5", "/lens/parity-lens/runs?offset=5")]
#[tokio::test]
async fn trailing_slashes_preserve_method_and_query(
    #[future(awt)] database: Database,
    #[case] path: &str,
    #[case] target: &str,
) {
    let server = serve(&database, true, true).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = client
        .get(server.url.join(path).unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 307);
    assert_eq!(
        response.headers()["location"],
        server.url.join(target).unwrap().as_str()
    );
}

struct ContendedRepository {
    lens: Lens,
    failures: usize,
    attempts: Arc<std::sync::atomic::AtomicUsize>,
}

impl LensRepository for ContendedRepository {
    async fn lenses(&self, _: &Scope) -> Result<Vec<Lens>, RepositoryError> {
        Ok(vec![self.lens.clone()])
    }
    async fn get(&self, _: &str) -> Result<Option<Lens>, RepositoryError> {
        Ok(Some(self.lens.clone()))
    }
    async fn create(&self, lens: &Lens) -> Result<Lens, RepositoryError> {
        Ok(lens.clone())
    }
    async fn replace(&self, _: &Lens, candidate: &Lens) -> Result<Lens, RepositoryError> {
        if self.attempts.fetch_add(1, Ordering::SeqCst) < self.failures {
            Err(RepositoryError::Conflict)
        } else {
            Ok(candidate.clone())
        }
    }
    async fn jobs(&self, _: &str, _: u64) -> Result<Vec<Job>, RepositoryError> {
        Ok(self.lens.jobs.clone())
    }
    async fn job(&self, _: &str, id: &str) -> Result<Option<Job>, RepositoryError> {
        Ok(self.lens.jobs.iter().find(|job| job.id == id).cloned())
    }
}

#[rstest]
#[case::last_attempt_can_succeed(39, 200)]
#[case::exhausted_attempts_return_conflict(40, 409)]
#[tokio::test]
async fn mutation_contention_has_the_existing_forty_attempt_budget(
    #[future(awt)] database: Database,
    seed: Lens,
    #[case] failures: usize,
    #[case] status: u16,
) {
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server = database
        .serve_router(false, |authentication| {
            lens_server::investigations::router(
                authentication,
                ContendedRepository {
                    lens: seed,
                    failures,
                    attempts: attempts.clone(),
                },
                Arc::new(Access {
                    configured: true,
                    worker: true,
                }),
            )
        })
        .await;
    let response = server
        .client
        .post(server.url.join("/lens/parity-lens/cancel").unwrap())
        .bearer_auth(ADMIN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), status);
    assert_eq!(attempts.load(Ordering::SeqCst), 40);
    let body: Value = response.json().await.unwrap();
    if status == 409 {
        assert_eq!(
            body,
            json!({"detail":"Lens changed concurrently; retry the operation"})
        );
    } else {
        assert_eq!(body["jobs"][0]["status"], "cancelled");
    }
}

struct WatchAccess(Arc<AtomicBool>);
impl InvestigationAccess for WatchAccess {
    fn tracing_enabled(&self) -> bool {
        true
    }
    async fn workers(&self, _: &Scope) -> Result<Vec<Worker>, RepositoryError> {
        Ok(Vec::new())
    }
    async fn validate_model(
        &self,
        settings: &LensSettings,
        _: &Identity,
    ) -> Result<(), InvestigationAccessError> {
        self.0.store(settings.enabled, Ordering::SeqCst);
        Ok(())
    }
    async fn validate_workers(
        &self,
        _: &LensSettings,
        _: &Scope,
    ) -> Result<(), InvestigationAccessError> {
        Ok(())
    }
}
