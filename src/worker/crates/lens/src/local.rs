mod inference;
mod job;
mod results;

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use lens_analysis::AnalysisModels;
use lens_contract::{
    investigations::{Lens, Worker},
    worker::Claim,
};
use lens_investigations::{
    LensRepository, RepositoryError, ScheduleRepository, WorkerRepository, can_access, claim_job,
    current_job, queue_job,
};
use litellm_storage_clickhouse::investigations::Investigations;
use tokio::sync::Semaphore;

use crate::{Error, SourceReader, control::JobClient};
use job::LocalJob;

#[derive(Clone)]
pub struct LocalControl {
    pub repository: Investigations,
    pub sources: SourceReader,
    pub models: Arc<AnalysisModels>,
    worker_hash: Arc<str>,
    slots: Arc<Semaphore>,
}

impl LocalControl {
    pub fn new(
        repository: Investigations,
        sources: SourceReader,
        models: Arc<AnalysisModels>,
        worker_hash: String,
    ) -> Self {
        Self {
            repository,
            sources,
            models,
            worker_hash: worker_hash.into(),
            slots: Arc::new(Semaphore::new(16)),
        }
    }

    async fn worker(&self) -> Result<Worker, Error> {
        self.repository
            .worker(&self.worker_hash)
            .await?
            .filter(|worker| !worker.revoked)
            .ok_or_else(|| rejected(401, "Worker credential is invalid or revoked"))
    }

    async fn update(
        &self,
        id: &str,
        transform: impl Fn(&Lens) -> Result<Lens, Error>,
    ) -> Result<Lens, Error> {
        self.repository
            .update_locked(id, transform)
            .await?
            .ok_or_else(|| rejected(404, "Lens not found"))
    }

    pub async fn claim(&self) -> Result<Option<Claim>, Error> {
        let worker = self.worker().await?;
        if worker.analysis_key_id.is_none() {
            return Ok(None);
        }
        let now = Utc::now();
        self.repository.heartbeat(&worker.id, now).await?;
        let models = self.models.models();
        let mut after = None;
        loop {
            let page = self
                .repository
                .due(&worker.scope, &now, 20, after.as_ref())
                .await?;
            for candidate in &page {
                let lens = &candidate.lens;
                let active = current_job(lens);
                let settings = active.map_or(&lens.settings, |job| &job.settings);
                if !can_access(&worker.scope, &lens.scope)
                    || !models.iter().any(|model| model == settings.model.as_str())
                {
                    continue;
                }
                let scheduled = if lens.settings.enabled && lens.next_run_at <= now {
                    queue_job(
                        lens,
                        now,
                        &uuid::Uuid::new_v4().to_string(),
                        Default::default(),
                    )?
                } else {
                    lens.clone()
                };
                let claimed = claim_job(&scheduled, &worker, now)?;
                match self.repository.replace(lens, &claimed).await {
                    Ok(updated) => {
                        if let Some(job) = current_job(&updated)
                            && job.worker_id.as_deref() == Some(&worker.id)
                            && job.status == crate::wire::JobStatus::Running
                            && active.is_none_or(|previous| {
                                previous.attempts != job.attempts || previous.id != job.id
                            })
                        {
                            return Ok(Some(Claim {
                                lens_id: updated.id.clone(),
                                job: job.clone(),
                                findings: updated.findings,
                                reviews: None,
                            }));
                        }
                    }
                    Err(RepositoryError::Conflict) => {}
                    Err(error) => return Err(error.into()),
                }
                self.repository.sync_due(lens).await?;
            }
            if page.len() < 20 {
                return Ok(None);
            }
            after = page.last().cloned();
        }
    }

    pub async fn run_once(&self) -> Result<bool, Error> {
        let Some(claim) = self.claim().await? else {
            return Ok(false);
        };
        let backend = LocalJob {
            control: self.clone(),
            lens_id: claim.lens_id.clone(),
            job_id: claim.job.id.clone(),
            attempt: claim.job.attempts,
            worker_id: claim.job.worker_id.clone().ok_or(Error::InvalidRequest)?,
        };
        let client = JobClient::local(
            Arc::new(backend),
            claim.job.settings.concurrency.get() as usize,
            self.slots.clone(),
        );
        crate::worker::execute(claim, client).await?;
        Ok(true)
    }

    async fn slot(&self) {
        let mut delay = 2;
        loop {
            match self.run_once().await {
                Ok(true) => {
                    delay = 2;
                    continue;
                }
                Ok(false) => {}
                Err(_) => tracing::warn!("Lens could not run a local investigation; retrying"),
            }
            tokio::time::sleep(Duration::from_secs(delay)).await;
            delay = (delay * 2).min(15);
        }
    }

    pub async fn serve(self) {
        tokio::join!(self.slot(), self.slot(), self.slot());
    }
}

impl lens_server::investigations::InvestigationAccess for LocalControl {
    fn tracing_enabled(&self) -> bool {
        true
    }

    async fn workers(
        &self,
        scope: &lens_contract::investigations::Scope,
    ) -> Result<Vec<Worker>, RepositoryError> {
        Ok(self
            .repository
            .workers()
            .await?
            .into_iter()
            .filter(|worker| can_access(scope, &worker.scope))
            .collect())
    }

    async fn validate_model(
        &self,
        settings: &crate::wire::LensSettings,
        _identity: &lens_contract::auth::Identity,
    ) -> Result<(), lens_server::investigations::InvestigationAccessError> {
        if self
            .models
            .models()
            .iter()
            .any(|model| model == settings.model.as_str())
        {
            return Ok(());
        }
        Err(
            lens_server::investigations::InvestigationAccessError::Rejected {
                status: http::StatusCode::BAD_REQUEST,
                reason: "Choose a configured Lens analysis model".into(),
            },
        )
    }

    async fn validate_workers(
        &self,
        settings: &crate::wire::LensSettings,
        scope: &lens_contract::investigations::Scope,
    ) -> Result<(), lens_server::investigations::InvestigationAccessError> {
        let worker = self.worker().await.map_err(|error| {
            lens_server::investigations::InvestigationAccessError::Unavailable(Box::new(error))
        })?;
        if can_access(&worker.scope, scope)
            && worker.analysis_key_id.is_some()
            && self
                .models
                .models()
                .iter()
                .any(|model| model == settings.model.as_str())
        {
            return Ok(());
        }
        Err(
            lens_server::investigations::InvestigationAccessError::Rejected {
                status: http::StatusCode::BAD_REQUEST,
                reason: "Configure an analysis provider in Lens before running an investigation"
                    .into(),
            },
        )
    }
}

fn rejected(status: u16, message: &str) -> Error {
    Error::Control {
        status,
        retry_after: None,
        diagnostic: Some(message.into()),
    }
}

fn backend_error(error: Error) -> Error {
    use lens_investigations::{CheckpointError, RepositoryError};
    match error {
        error @ Error::Control { .. } | error @ Error::Request(_) => error,
        Error::InvestigationStorage(RepositoryError::Conflict)
        | Error::Checkpoint(CheckpointError::Store(RepositoryError::Conflict)) => {
            rejected(409, "Lens changed concurrently; retry the operation")
        }
        Error::Checkpoint(CheckpointError::Ownership) => {
            rejected(409, "This worker no longer owns the job")
        }
        Error::Investigation(error) | Error::Checkpoint(CheckpointError::Invalid(error)) => {
            rejected(422, &error.to_string())
        }
        Error::StateStorage(litellm_storage_clickhouse::Error::ResponseTooLarge) => {
            rejected(413, "ClickHouse query exceeded the response size limit")
        }
        error => rejected(error.status().as_u16(), &error.to_string()),
    }
}

#[cfg(test)]
mod fixtures {
    use super::*;
    use crate::{State, Storage, config::http_client};
    use lens_analysis::{Deployment, Provider, Secret};
    use lens_contract::{investigations::Scope, worker::LensSettings};
    use lens_inference::{ModelCapacity, OutputLimits};
    use litellm_storage_clickhouse::{Connection, execute_statement, state::ClickHouseState};
    use rstest::fixture;
    use serde_json::json;
    use testcontainers_modules::{
        clickhouse::ClickHouse,
        testcontainers::{ContainerAsync, ImageExt, runners::AsyncRunner},
    };

    pub(super) struct StoredJob {
        pub job: LocalJob,
        pub provider: wiremock::MockServer,
        container: Option<ContainerAsync<ClickHouse>>,
        connection: Connection,
        database: String,
    }

    impl Drop for StoredJob {
        fn drop(&mut self) {
            if self.container.is_some() {
                return;
            }
            let connection = self.connection.clone();
            let database = self.database.clone();
            std::thread::spawn(move || {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap()
                    .block_on(async {
                        execute_statement(
                            &http_client().unwrap(),
                            &connection,
                            &format!("DROP DATABASE `{database}` SYNC"),
                            Duration::from_secs(30),
                        )
                        .await
                        .unwrap();
                    });
            })
            .join()
            .unwrap();
        }
    }

    #[fixture]
    pub(super) async fn stored_job() -> StoredJob {
        let (container, url) = match std::env::var("CLICKHOUSE_STATE_TEST_URL") {
            Ok(url) => (None, url),
            Err(_) => {
                let container = ClickHouse::default()
                    .with_tag("26.9.6.6@sha256:eb4870e7ca7ed70c259eebfcfbee6cf797017f6b5436c2926bbbfe3d4d28486e")
                    .with_env_var("CLICKHOUSE_SKIP_USER_SETUP", "1")
                    .with_copy_to("/etc/clickhouse-server/config.d/lens-keeper.xml", include_bytes!("../../storage-clickhouse/tests/state/fixtures/keeper.xml").to_vec())
                    .start().await.unwrap();
                let url = format!(
                    "http://{}:{}",
                    container.get_host().await.unwrap(),
                    container.get_host_port_ipv4(8123).await.unwrap()
                );
                (Some(container), url)
            }
        };
        let database = format!("lens_local_job_{}", uuid::Uuid::new_v4().simple());
        let connection = Connection::writer(&url).unwrap();
        let client = http_client().unwrap();
        execute_statement(
            &client,
            &connection,
            &format!("CREATE DATABASE `{database}`"),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        let config =
            litellm_traces_clickhouse::Config::new(database.clone(), &url, 14, 65_536).unwrap();
        let store = ClickHouseState::new(client.clone(), config.storage().reader().clone());
        store
            .initialize(&format!("/local-job-tests/{database}"))
            .await
            .unwrap();
        let repository = Investigations(store);
        repository.initialize().await.unwrap();
        let provider = wiremock::MockServer::start().await;
        let deployment = Deployment {
            name: "test-analysis".into(),
            model: "gpt-4o-mini".into(),
            provider: Provider::OpenAiCompatible,
            api_base: Some(format!("{}/v1", provider.uri()).parse().unwrap()),
            api_key: Secret::new("private-fixture-provider-key"),
            input_cost_per_token: Some(0.000001),
            output_cost_per_token: Some(0.000002),
            capacity: ModelCapacity {
                max_input_tokens: Some(1_000_000.try_into().unwrap()),
                max_output_tokens: Some(4096.try_into().unwrap()),
            },
            output_limits: OutputLimits {
                max_tokens: Some(1024.try_into().unwrap()),
                ..Default::default()
            },
        };
        let models = AnalysisModels::new(
            lens_analysis::bundled_catalog().unwrap(),
            vec![deployment],
            Default::default(),
        )
        .unwrap();
        let worker_hash = "a".repeat(64);
        let worker = Worker {
            id: "worker".into(),
            name: "Local test worker".into(),
            scope: Scope {
                all_teams: true,
                ..Default::default()
            },
            last_seen: Utc::now(),
            revoked: false,
            analysis_key_id: Some(worker_hash.clone()),
        };
        repository
            .configure_service_worker(&worker, &worker_hash)
            .await
            .unwrap();
        let sources = SourceReader(Arc::new(State::standalone(Storage::new(
            config,
            client,
            "fixture-service-secret".into(),
        ))));
        let control = LocalControl::new(repository, sources, Arc::new(models), worker_hash);
        let settings: LensSettings = serde_json::from_value(json!({"name":"Local job contract", "model":"test-analysis", "monthly_budget":10, "checks":[{"id":"correct","instruction":"The response must match the evidence"}]})).unwrap();
        let lens = lens_investigations::create_lens(
            settings,
            Scope {
                all_teams: true,
                ..Default::default()
            },
            Utc::now(),
            "lens",
            "job",
        )
        .unwrap();
        control.repository.create(&lens).await.unwrap();
        let claim = control.claim().await.unwrap().unwrap();
        let job = LocalJob {
            control,
            lens_id: claim.lens_id,
            job_id: claim.job.id,
            attempt: claim.job.attempts,
            worker_id: claim.job.worker_id.unwrap(),
        };
        StoredJob {
            job,
            provider,
            container,
            connection,
            database,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{control::JobBackend, wire};
    use futures_util::future::BoxFuture;
    use lens_investigations::CheckpointError;
    use rstest::rstest;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    fn unavailable() -> Error {
        CheckpointError::Store(RepositoryError::Unavailable(Box::new(
            std::io::Error::from(std::io::ErrorKind::ConnectionReset),
        )))
        .into()
    }

    #[rstest]
    #[case::checkpoint_outage(unavailable(), 503, true)]
    #[case::state_outage(
        Error::StateStorage(litellm_storage_clickhouse::Error::QueryFailed(503)),
        503,
        true
    )]
    #[case::checkpoint_conflict(CheckpointError::Store(RepositoryError::Conflict).into(), 409, false)]
    #[case::lost_ownership(CheckpointError::Ownership.into(), 409, false)]
    #[case::invalid_progress(CheckpointError::Invalid(lens_investigations::Error::ReviewCount).into(), 422, false)]
    #[case::oversized_response(
        Error::StateStorage(litellm_storage_clickhouse::Error::ResponseTooLarge),
        413,
        false
    )]
    fn local_errors_preserve_control_failure_semantics(
        #[case] input: Error,
        #[case] status: u16,
        #[case] retryable: bool,
    ) {
        let error = backend_error(input);
        assert!(error.is_control_failure());
        assert_eq!(error.retryable(), retryable);
        assert!(matches!(error, Error::Control { status: actual, .. } if actual == status));
    }

    struct WaitingJob {
        sample_entered: Notify,
        sample_ready: Notify,
        pulse_seen: Notify,
        pulse_calls: AtomicUsize,
        revoked: bool,
        results: Mutex<Vec<wire::Result>>,
    }

    impl JobBackend for WaitingJob {
        fn sample(&self) -> BoxFuture<'_, Result<wire::Sample, Error>> {
            Box::pin(async move {
                self.sample_entered.notify_one();
                self.sample_ready.notified().await;
                Ok(serde_json::from_value(
                    serde_json::json!({"executions":[],"eligible":0,"selected":0}),
                )
                .unwrap())
            })
        }

        fn reviews(&self) -> BoxFuture<'_, Result<Vec<wire::Review>, Error>> {
            Box::pin(async { Ok(vec![]) })
        }

        fn content<'a>(
            &'a self,
            _: &'a str,
            _: &'a str,
            _: usize,
        ) -> BoxFuture<'a, Result<wire::ExecutionContent, Error>> {
            Box::pin(async { Err(Error::InvalidRequest) })
        }

        fn model<'a>(
            &'a self,
            _: &'a wire::ModelRequest,
        ) -> BoxFuture<'a, Result<wire::ModelResult, Error>> {
            Box::pin(async { Err(Error::InvalidRequest) })
        }

        fn progress<'a>(&'a self, _: &'a wire::Progress) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                let call = self.pulse_calls.fetch_add(1, Ordering::SeqCst);
                self.pulse_seen.notify_one();
                if self.revoked {
                    return Err(backend_error(CheckpointError::Ownership.into()));
                }
                if call == 0 {
                    return Err(backend_error(unavailable()));
                }
                Ok(())
            })
        }

        fn finish<'a>(&'a self, result: &'a wire::Result) -> BoxFuture<'a, Result<(), Error>> {
            Box::pin(async move {
                self.results.lock().unwrap().push(result.clone());
                Ok(())
            })
        }
    }

    #[rstest]
    #[case::storage_recovers(false)]
    #[case::ownership_revoked(true)]
    #[tokio::test(start_paused = true)]
    async fn heartbeat_storage_failure_keeps_work_alive_but_revocation_stops_it(
        #[case] revoked: bool,
    ) {
        let backend = Arc::new(WaitingJob {
            sample_entered: Notify::new(),
            sample_ready: Notify::new(),
            pulse_seen: Notify::new(),
            pulse_calls: AtomicUsize::new(0),
            revoked,
            results: Mutex::new(vec![]),
        });
        let client = JobClient::local(backend.clone(), 1, Arc::new(Semaphore::new(1)));
        let claim = serde_json::from_str(include_str!("../tests/fixtures/claim.json")).unwrap();
        let task = tokio::spawn(crate::worker::execute(claim, client));
        backend.sample_entered.notified().await;
        tokio::time::advance(Duration::from_secs(30)).await;
        backend.pulse_seen.notified().await;
        if revoked {
            task.await.unwrap().unwrap();
            assert!(backend.results.lock().unwrap().is_empty());
            return;
        }
        assert!(!task.is_finished());
        tokio::time::advance(Duration::from_secs(30)).await;
        backend.pulse_seen.notified().await;
        assert!(!task.is_finished());
        backend.sample_ready.notify_one();
        task.await.unwrap().unwrap();
        let results = backend.results.lock().unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].error.is_empty());
    }
}
