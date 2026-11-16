use std::{num::NonZeroU64, sync::Arc};

use chrono::Utc;
use lens_analysis::{PrepareOptions, ResponseSchema};
use lens_contract::{
    investigations::BudgetReservation,
    worker::{ModelRequest, ModelResult, ModelResultFinishReason},
};
use lens_inference::{
    BUDGET_LEASE, BUDGET_RENEW_INTERVAL, BUDGET_WAIT_TIMEOUT, model_step, renew_reservation,
    reserve_attempt, settle_amount,
};

use super::{LocalControl, job::LocalJob, rejected};
use crate::Error;

trait Settlement: Clone + Send + Sync + 'static {
    fn settle(
        &self,
        lens_id: &str,
        id: &str,
        cost: f64,
        step: Option<&crate::wire::Step>,
    ) -> impl std::future::Future<Output = Result<(), Error>> + Send;
}

impl Settlement for LocalControl {
    async fn settle(
        &self,
        lens_id: &str,
        id: &str,
        cost: f64,
        step: Option<&crate::wire::Step>,
    ) -> Result<(), Error> {
        let id = id.to_owned();
        let step = step.cloned();
        self.update(lens_id, move |lens| {
            Ok(settle_amount(lens, &id, cost, step.as_ref()))
        })
        .await?;
        Ok(())
    }
}

struct Reservation<S: Settlement> {
    control: S,
    lens_id: String,
    id: String,
    cost: f64,
    step: Option<crate::wire::Step>,
    armed: bool,
}

impl<S: Settlement> Reservation<S> {
    async fn settle(&mut self) -> Result<(), Error> {
        self.control
            .settle(&self.lens_id, &self.id, self.cost, self.step.as_ref())
            .await?;
        self.armed = false;
        Ok(())
    }

    async fn release(&mut self) {
        if self
            .control
            .settle(&self.lens_id, &self.id, 0.0, None)
            .await
            .is_err()
        {
            tracing::warn!("Lens budget cleanup failed; the reservation will expire");
        }
        self.armed = false;
    }
}

impl<S: Settlement> Drop for Reservation<S> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let control = self.control.clone();
        let lens_id = self.lens_id.clone();
        let id = self.id.clone();
        let cost = self.cost;
        let step = self.step.clone();
        runtime.spawn(async move {
            if control
                .settle(&lens_id, &id, cost, step.as_ref())
                .await
                .is_err()
            {
                tracing::warn!(
                    "Lens cancelled-request budget cleanup failed; the reservation will expire"
                );
            }
        });
    }
}

impl LocalJob {
    pub(super) async fn analyze(
        &self,
        body: &ModelRequest,
        schema: Option<ResponseSchema<'_>>,
    ) -> Result<ModelResult, Error> {
        let (mut result, mut larger) = self.complete(body, schema, None).await?;
        while let Some(minimum) = larger {
            match self.complete(body, schema, Some(minimum)).await {
                Ok(next) => (result, larger) = next,
                Err(Error::Control { status: 402, .. }) => break,
                Err(error) => return Err(error),
            }
        }
        Ok(result)
    }

    async fn complete(
        &self,
        body: &ModelRequest,
        schema: Option<ResponseSchema<'_>>,
        minimum_output_tokens: Option<NonZeroU64>,
    ) -> Result<(ModelResult, Option<NonZeroU64>), Error> {
        let (_, job) = self.assigned().await?;
        let job = Arc::new(job);
        let models = self.control.models.get();
        let prepared = models
            .prepare_with(
                job.settings.model.as_str(),
                body,
                PrepareOptions {
                    response_schema: schema,
                    minimum_output_tokens,
                },
            )
            .await
            .map_err(provider_error)?;
        if prepared.context_exceeded {
            return Ok((context_exceeded(), None));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let mut reservation = Reservation {
            control: self.control.clone(),
            lens_id: self.lens_id.clone(),
            id: id.clone(),
            cost: 0.0,
            step: None,
            armed: true,
        };
        let wait = async {
            loop {
                let hold_id = id.clone();
                let job = job.clone();
                let worker_id = self.worker_id.clone();
                let amount = prepared.estimate;
                let reserved = self
                    .control
                    .update(&self.lens_id, move |lens| {
                        let now = Utc::now();
                        let hold = BudgetReservation {
                            id: hold_id.clone(),
                            job_id: job.id.clone(),
                            amount,
                            month: now.format("%Y-%m").to_string(),
                            expires_at: Some(now + BUDGET_LEASE),
                        };
                        reserve_attempt(lens, &job, &worker_id, &hold, now).map_err(policy_error)
                    })
                    .await?;
                if reserved.reservations.iter().any(|held| held.id == id) {
                    return Ok::<_, Error>(());
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        };
        match tokio::time::timeout(BUDGET_WAIT_TIMEOUT, wait).await {
            Ok(Ok(())) => {}
            result => {
                reservation.release().await;
                return Err(match result {
                    Ok(Err(error)) => error,
                    _ => rejected(504, "Analysis request timed out waiting for budget"),
                });
            }
        }
        let renew = async {
            loop {
                tokio::time::sleep(BUDGET_RENEW_INTERVAL).await;
                let held = id.clone();
                match tokio::time::timeout(
                    BUDGET_RENEW_INTERVAL,
                    self.control.update(&self.lens_id, move |lens| {
                        renew_reservation(lens, &held, Utc::now()).map_err(policy_error)
                    }),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => return Err(error),
                    Err(_) => {
                        return Err(rejected(
                            503,
                            "Analysis budget reservation renewal timed out",
                        ));
                    }
                }
            }
        };
        let completion = tokio::select! {
            result=models.complete(&prepared)=>result.map_err(provider_error),
            result=renew=>result,
        };
        let completion = match completion {
            Ok(completion) => completion,
            Err(error) => {
                reservation.release().await;
                return Err(error);
            }
        };
        if completion.result.context_exceeded {
            reservation.release().await;
            return Ok((completion.result, None));
        }
        reservation.cost = completion.result.cost;
        let step = model_step(
            &completion.usage,
            body,
            job.settings.model.as_str(),
            completion.result.cost,
            Utc::now(),
        )?;
        reservation.step = Some(step.clone());
        reservation.settle().await?;
        let larger = (completion.result.finish_reason == Some(ModelResultFinishReason::Length))
            .then(|| prepared.larger_output())
            .flatten();
        Ok((completion.result, larger))
    }
}

fn context_exceeded() -> ModelResult {
    ModelResult {
        content: String::new(),
        cost: 0.0,
        context_exceeded: true,
        finish_reason: None,
    }
}

fn policy_error(error: lens_inference::Error) -> Error {
    let status = match error {
        lens_inference::Error::MonthlyBudget | lens_inference::Error::RequestBudget { .. } => 402,
        lens_inference::Error::JobReassigned => return Error::JobOwnershipLost,
        lens_inference::Error::ReservationExpired => 503,
        _ => 400,
    };
    rejected(status, &error.to_string())
}

fn provider_error(error: lens_analysis::Error) -> Error {
    match error {
        lens_analysis::Error::Provider {
            status,
            retry_after,
            diagnostic,
        } => {
            tracing::warn!(
                upstream_status = status,
                classification = %diagnostic.classification,
                request_id = diagnostic.request_id.as_deref(),
                "Analysis provider rejected the request"
            );
            Error::Control {
                status,
                retry_after,
                diagnostic: Some(diagnostic.to_string()),
            }
        }
        lens_analysis::Error::Timeout => {
            rejected(504, "Analysis request timed out waiting for model output")
        }
        lens_analysis::Error::Transport(error) => Error::Request(error),
        lens_analysis::Error::Policy(error) => policy_error(error),
        error => Error::Analysis(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::fixtures::{StoredJob, stored_job};
    use lens_contract::{investigations::Lens, worker::Step};
    use lens_investigations::LensRepository;
    use rstest::{fixture, rstest};
    use serde_json::json;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    #[derive(Clone)]
    struct Ledger {
        lens: Arc<Mutex<Lens>>,
        calls: Arc<AtomicUsize>,
        entered: Arc<Notify>,
        completed: Arc<Notify>,
        commit_before_wait: bool,
        wait_on_first: bool,
    }

    impl Ledger {
        fn commit(&self, id: &str, cost: f64, step: Option<&Step>) {
            let mut lens = self.lens.lock().unwrap();
            *lens = settle_amount(&lens, id, cost, step);
        }
    }

    impl Settlement for Ledger {
        async fn settle(
            &self,
            lens_id: &str,
            id: &str,
            cost: f64,
            step: Option<&Step>,
        ) -> Result<(), Error> {
            assert_eq!(lens_id, "lens");
            let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
            if self.commit_before_wait {
                self.commit(id, cost, step);
            }
            if first && self.wait_on_first {
                self.entered.notify_one();
                std::future::pending::<()>().await;
            }
            self.commit(id, cost, step);
            self.completed.notify_one();
            Ok(())
        }
    }

    #[fixture]
    fn ledger() -> Ledger {
        let at = "2026-01-15T12:00:00Z";
        let lens = serde_json::from_value(json!({
            "id":"lens", "scope":{"team_id":"team"},
            "settings":{"name":"Investigation","model":"analysis","monthly_budget":10},
            "created_at":at,"next_run_at":at,"budget_month":"2026-01",
            "reservations":[{"id":"request","job_id":"job","amount":1,"month":"2026-01","expires_at":null}],
            "jobs":[{"id":"job","created_at":at,"start":at,"end":at,"revision":1,
                "settings":{"name":"Investigation","model":"analysis","monthly_budget":10},
                "status":"running","worker_id":"worker","attempts":1}]
        })).unwrap();
        Ledger {
            lens: Arc::new(Mutex::new(lens)),
            calls: Arc::new(AtomicUsize::new(0)),
            entered: Arc::new(Notify::new()),
            completed: Arc::new(Notify::new()),
            commit_before_wait: false,
            wait_on_first: true,
        }
    }

    fn reservation(ledger: Ledger, cost: f64) -> Reservation<Ledger> {
        Reservation {
            control: ledger,
            lens_id: "lens".into(),
            id: "request".into(),
            cost,
            step: (cost > 0.0).then(|| {
                serde_json::from_value(json!({
                "at":"2026-01-15T12:00:00Z", "kind":"model", "label":"Reviewed a run", "cost":cost
            })).unwrap()
            }),
            armed: true,
        }
    }

    #[rstest]
    #[case::before_commit(false)]
    #[case::after_commit(true)]
    #[tokio::test]
    async fn cancelling_paid_settlement_preserves_cost_exactly_once(
        ledger: Ledger,
        #[case] committed: bool,
    ) {
        let ledger = Ledger {
            commit_before_wait: committed,
            ..ledger
        };
        let mut reservation = reservation(ledger.clone(), 0.25);
        let task = tokio::spawn(async move { reservation.settle().await });
        ledger.entered.notified().await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            ledger.completed.notified(),
        )
        .await
        .unwrap();
        let lens = ledger.lens.lock().unwrap();
        assert_eq!(lens.spent, 0.25);
        assert_eq!(lens.jobs[0].cost, 0.25);
        assert_eq!(lens.jobs[0].steps.len(), 1);
        assert!(lens.reservations.is_empty());
        assert_eq!(ledger.calls.load(Ordering::SeqCst), 2);
    }

    #[rstest]
    #[tokio::test]
    async fn cancellation_before_provider_response_releases_without_charge(ledger: Ledger) {
        let ledger = Ledger {
            wait_on_first: false,
            ..ledger
        };
        drop(reservation(ledger.clone(), 0.0));
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            ledger.completed.notified(),
        )
        .await
        .unwrap();
        let lens = ledger.lens.lock().unwrap();
        assert_eq!(lens.spent, 0.0);
        assert!(lens.jobs[0].steps.is_empty());
        assert!(lens.reservations.is_empty());
    }

    #[rstest]
    #[tokio::test]
    async fn completed_settlement_disarms_cleanup(ledger: Ledger) {
        let ledger = Ledger {
            wait_on_first: false,
            ..ledger
        };
        let mut reservation = reservation(ledger.clone(), 0.25);
        reservation.settle().await.unwrap();
        drop(reservation);
        tokio::task::yield_now().await;
        assert_eq!(ledger.calls.load(Ordering::SeqCst), 1);
        assert_eq!(ledger.lens.lock().unwrap().spent, 0.25);
    }

    #[rstest]
    #[tokio::test]
    async fn releasing_reservation_finishes_cleanup_before_returning(ledger: Ledger) {
        let ledger = Ledger {
            wait_on_first: false,
            ..ledger
        };
        let mut reservation = reservation(ledger.clone(), 0.25);
        reservation.release().await;
        {
            let lens = ledger.lens.lock().unwrap();
            assert!(lens.reservations.is_empty());
            assert_eq!(lens.spent, 0.0);
            assert_eq!(lens.jobs[0].cost, 0.0);
            assert!(lens.jobs[0].steps.is_empty());
        }
        drop(reservation);
        tokio::task::yield_now().await;
        assert_eq!(ledger.calls.load(Ordering::SeqCst), 1);
    }

    #[rstest]
    #[case::monthly_budget(lens_inference::Error::MonthlyBudget, 402, false)]
    #[case::request_budget(lens_inference::Error::RequestBudget { amount: 2.0, available: 1.0 }, 402, false)]
    #[case::expired(lens_inference::Error::ReservationExpired, 503, true)]
    #[case::invalid_prompt(lens_inference::Error::MalformedPrompt, 400, false)]
    fn inference_policy_preserves_status_and_diagnostic(
        #[case] input: lens_inference::Error,
        #[case] expected_status: u16,
        #[case] retryable: bool,
    ) {
        let expected_diagnostic = input.to_string();
        let error = policy_error(input);
        assert_eq!(error.retryable(), retryable);
        let Error::Control {
            status, diagnostic, ..
        } = error
        else {
            panic!("inference policy must return a control failure");
        };
        assert_eq!(status, expected_status);
        assert_eq!(diagnostic.as_deref(), Some(expected_diagnostic.as_str()));
    }

    #[rstest]
    fn reassigned_inference_is_confirmed_ownership_loss() {
        let error = policy_error(lens_inference::Error::JobReassigned);
        assert!(matches!(error, Error::JobOwnershipLost));
        assert!(error.is_control_failure());
        assert!(!error.retryable());
    }

    #[rstest]
    #[case::unavailable(503, true)]
    #[case::rate_limit(429, true)]
    #[case::authentication(401, false)]
    fn provider_failure_preserves_safe_diagnostics_and_retry_policy(
        #[case] upstream_status: u16,
        #[case] retryable: bool,
    ) {
        let provider_diagnostic = lens_analysis::ProviderDiagnostic {
            classification: lens_analysis::ProviderFailureKind::Unavailable,
            request_id: Some("req_diagnostic12345".into()),
        };
        let expected = provider_diagnostic.to_string();
        let error = provider_error(lens_analysis::Error::Provider {
            status: upstream_status,
            retry_after: Some(17),
            diagnostic: provider_diagnostic,
        });

        assert_eq!(error.retryable(), retryable);
        let Error::Control {
            status,
            retry_after,
            diagnostic,
        } = error
        else {
            panic!("Provider failures must retain their control status");
        };
        assert_eq!(status, upstream_status);
        assert_eq!(retry_after, Some(17));
        assert_eq!(diagnostic.as_deref(), Some(expected.as_str()));
    }

    #[rstest]
    #[case::monthly_budget(0.0)]
    #[case::request_budget(0.00000001)]
    #[tokio::test]
    async fn rejected_budget_preserves_policy_status_without_calling_provider(
        #[future(awt)] stored_job: StoredJob,
        #[case] budget: f64,
    ) {
        let lens = stored_job.job.lens().await.unwrap();
        let mut denied = lens.clone();
        denied.settings.monthly_budget = budget;
        stored_job
            .job
            .control
            .repository
            .replace(&lens, &denied)
            .await
            .unwrap();
        let request = crate::model::request(
            crate::wire::ModelRequestPurpose::Extract,
            json!({"task":"Review the run"}),
        )
        .unwrap();
        let error = stored_job.job.analyze(&request, None).await.unwrap_err();
        assert!(
            matches!(error, Error::Control { status: 402, .. }),
            "{error}"
        );
        assert!(
            stored_job
                .provider
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
        let persisted = stored_job.job.lens().await.unwrap();
        assert!(persisted.reservations.is_empty());
        assert_eq!(persisted.spent, 0.0);
    }

    fn review_request() -> ModelRequest {
        crate::model::request(
            crate::wire::ModelRequestPurpose::Extract,
            json!({"task":"Review the run"}),
        )
        .unwrap()
    }

    async fn serve_replies(stored_job: &StoredJob, replies: Vec<(&'static str, &'static str)>) {
        let served = AtomicUsize::new(0);
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(move |_: &wiremock::Request| {
                let (finish_reason, content) = replies[served.fetch_add(1, Ordering::SeqCst)];
                wiremock::ResponseTemplate::new(200).set_body_json(json!({
                    "model": "gpt-4o-mini",
                    "choices": [{"message": {"content": content}, "finish_reason": finish_reason}],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 5},
                }))
            })
            .mount(&stored_job.provider)
            .await;
    }

    async fn requested_allowances(stored_job: &StoredJob) -> Vec<serde_json::Value> {
        stored_job
            .provider
            .received_requests()
            .await
            .unwrap()
            .iter()
            .map(|request| request.body_json::<serde_json::Value>().unwrap()["max_tokens"].clone())
            .collect()
    }

    #[rstest]
    #[case::completes_with_more_room(vec![("length", "{\"result\":"), ("stop", "{\"ok\":true}")], None, "{\"ok\":true}")]
    #[case::still_cut_off_at_capacity(vec![("length", "{\"a\":"), ("length", "{\"a\":1,\"b\":")], Some(ModelResultFinishReason::Length), "{\"a\":1,\"b\":")]
    #[tokio::test]
    async fn truncated_response_is_retried_once_per_larger_allowance_and_every_call_is_billed(
        #[future(awt)] stored_job: StoredJob,
        #[case] replies: Vec<(&'static str, &'static str)>,
        #[case] finish_reason: Option<ModelResultFinishReason>,
        #[case] content: &str,
    ) {
        serve_replies(&stored_job, replies).await;
        let result = stored_job
            .job
            .analyze(&review_request(), None)
            .await
            .unwrap();
        assert_eq!(result.content, content);
        assert_eq!(result.finish_reason, finish_reason);
        assert_eq!(requested_allowances(&stored_job).await, [1024, 4096]);
        let lens = stored_job.job.lens().await.unwrap();
        assert!(lens.reservations.is_empty());
        assert_eq!(lens.jobs[0].steps.len(), 2);
        assert!((lens.spent - 2.0 * (10.0 * 0.000001 + 5.0 * 0.000002)).abs() < 1e-12);
    }

    #[rstest]
    #[tokio::test]
    async fn larger_allowance_the_budget_cannot_cover_returns_the_truncated_response(
        #[future(awt)] stored_job: StoredJob,
    ) {
        let lens = stored_job.job.lens().await.unwrap();
        let mut limited = lens.clone();
        limited.settings.monthly_budget = 0.005;
        stored_job
            .job
            .control
            .repository
            .replace(&lens, &limited)
            .await
            .unwrap();
        serve_replies(&stored_job, vec![("length", "{\"result\":")]).await;
        let result = stored_job
            .job
            .analyze(&review_request(), None)
            .await
            .unwrap();
        assert_eq!(result.finish_reason, Some(ModelResultFinishReason::Length));
        assert_eq!(requested_allowances(&stored_job).await, [1024]);
        assert!(stored_job.job.lens().await.unwrap().reservations.is_empty());
    }

    #[rstest]
    #[tokio::test]
    async fn provider_call_holds_a_live_lease_and_cancellation_releases_it(
        #[future(awt)] stored_job: StoredJob,
    ) {
        let entered = Arc::new(Notify::new());
        let observed = entered.clone();
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/v1/chat/completions"))
            .respond_with(move |_: &wiremock::Request| {
                observed.notify_one();
                wiremock::ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(30))
            })
            .expect(1)
            .mount(&stored_job.provider)
            .await;
        let job = stored_job.job.clone();
        let before = Utc::now();
        let task = tokio::spawn(async move {
            let request = crate::model::request(
                crate::wire::ModelRequestPurpose::Extract,
                json!({"task":"Review the run"}),
            )
            .unwrap();
            job.analyze(&request, None).await
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), entered.notified())
            .await
            .unwrap();
        let current = stored_job.job.lens().await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(current.reservations.len(), 1);
        let hold = &current.reservations[0];
        assert_eq!(hold.job_id, stored_job.job.job_id);
        assert!(hold.amount > 0.0);
        assert!(hold.expires_at.unwrap() >= before + BUDGET_LEASE);
        assert!(hold.expires_at.unwrap() <= Utc::now() + BUDGET_LEASE);
        let released = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let lens = stored_job.job.lens().await.unwrap();
                if lens.reservations.is_empty() {
                    break lens;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(released.spent, 0.0);
        assert_eq!(released.jobs[0].cost, 0.0);
    }
}
