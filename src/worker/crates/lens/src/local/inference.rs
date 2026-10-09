use chrono::Utc;
use lens_contract::{
    investigations::BudgetReservation,
    worker::{ModelRequest, ModelResult},
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
        self.update(lens_id, |lens| Ok(settle_amount(lens, id, cost, step)))
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
    pub(super) async fn analyze(&self, body: &ModelRequest) -> Result<ModelResult, Error> {
        let (_, job) = self.assigned().await?;
        let prepared = self
            .control
            .models
            .prepare(job.settings.model.as_str(), body)
            .await
            .map_err(provider_error)?;
        if prepared.context_exceeded {
            return Ok(context_exceeded());
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
                let reserved = self
                    .control
                    .update(&self.lens_id, |lens| {
                        let now = Utc::now();
                        let hold = BudgetReservation {
                            id: id.clone(),
                            job_id: job.id.clone(),
                            amount: prepared.estimate,
                            month: now.format("%Y-%m").to_string(),
                            expires_at: Some(now + BUDGET_LEASE),
                        };
                        reserve_attempt(lens, &job, &self.worker_id, &hold, now)
                            .map_err(policy_error)
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
                match tokio::time::timeout(
                    BUDGET_RENEW_INTERVAL,
                    self.control.update(&self.lens_id, |lens| {
                        renew_reservation(lens, &id, Utc::now()).map_err(policy_error)
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
            result=self.control.models.complete(&prepared)=>result.map_err(provider_error),
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
            return Ok(completion.result);
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
        Ok(completion.result)
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
        lens_inference::Error::JobReassigned => 409,
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
        } => Error::Control {
            status,
            retry_after,
            diagnostic: Some("Analysis provider rejected the request".into()),
        },
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
    use lens_contract::{investigations::Lens, worker::Step};
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
}
