use std::collections::BTreeSet;

use chrono::{DateTime, TimeDelta, Utc};
use lens_contract::{
    investigations::{FindingRun, Lens},
    worker::{Coverage, Job, JobStatus, Result as InvestigationResult, Sample},
};
use lens_investigations::{
    analysis_checks, end_job, merge_finding, next_scan_start, replace_job, result_status,
};

use super::{job::LocalJob, rejected};
use crate::Error;

impl LocalJob {
    pub(super) async fn save_result(&self, body: &InvestigationResult) -> Result<(), Error> {
        let lens = self.lens().await?;
        if let Some(old) = lens.jobs.iter().find(|job| job.id == self.job_id)
            && matches!(old.status, JobStatus::Completed | JobStatus::Failed)
            && old.worker_id.as_deref() == Some(&self.worker_id)
            && old.attempts == self.attempt
        {
            if old.status == JobStatus::Completed && !old.review_versions.is_empty() {
                self.control
                    .repository
                    .complete_reviews(&self.lens_id, old, &old.review_versions)
                    .await?;
            }
            return Ok(());
        }
        let job = self.active(&lens, Utc::now())?.clone();
        let empty = Sample {
            eligible: 0,
            selected: 0,
            executions: vec![],
            next_cursor: None,
            next_offset: None,
        };
        let sample = job.sample.as_ref().unwrap_or(&empty);
        validate(&job, sample, body)?;
        for finding in &body.findings {
            if let Some(id) = &finding.existing_finding_id
                && !lens.findings.iter().any(|prior| {
                    &prior.id == id && prior.kind.to_string() == finding.kind.to_string()
                })
            {
                return Err(rejected(
                    422,
                    "Existing finding must belong to the same kind",
                ));
            }
            if finding.merged_finding_ids.iter().any(|id| {
                !lens.findings.iter().any(|prior| {
                    &prior.id == id && prior.kind.to_string() == finding.kind.to_string()
                })
            }) {
                return Err(rejected(
                    422,
                    "Merged finding must belong to this investigation and kind",
                ));
            }
            for evidence in &finding.evidence {
                let execution = sample
                    .executions
                    .iter()
                    .find(|execution| execution.id == evidence.execution_id)
                    .ok_or_else(|| rejected(422, "Finding references evidence outside the job"))?;
                if !self
                    .control
                    .sources
                    .verify_evidence(&lens.scope, execution, evidence)
                    .await?
                {
                    return Err(rejected(
                        422,
                        "Evidence quote does not match stored content",
                    ));
                }
            }
        }
        let ids = lens
            .findings
            .iter()
            .filter(|finding| finding.investigation_runs.is_empty())
            .map(|finding| finding.id.clone())
            .collect::<Vec<_>>();
        let historical = self
            .control
            .repository
            .finding_runs(&self.lens_id, &ids)
            .await?;
        let finished = self
            .control
            .update(&self.lens_id, |current| {
                let now = Utc::now();
                let Ok(active) = self.active(current, now) else {
                    return Ok(current.clone());
                };
                finish(current, active, body, &historical, now)
            })
            .await?;
        if !body.review_versions.is_empty()
            && finished.jobs.iter().any(|job| {
                job.id == self.job_id
                    && job.status == JobStatus::Completed
                    && job.attempts == self.attempt
                    && job.worker_id.as_deref() == Some(&self.worker_id)
            })
        {
            self.control
                .repository
                .complete_reviews(&self.lens_id, &job, &body.review_versions)
                .await?;
        }
        Ok(())
    }
}

fn validate(job: &Job, sample: &Sample, body: &InvestigationResult) -> Result<(), Error> {
    let allowed: BTreeSet<_> = sample
        .executions
        .iter()
        .map(|execution| execution.id.as_str())
        .collect();
    let assessed: BTreeSet<_> = body
        .assessments
        .iter()
        .map(|assessment| assessment.execution_id.as_str())
        .collect();
    if assessed.len() != body.assessments.len() {
        return Err(rejected(422, "Each run must have one assessment"));
    }
    if !assessed.is_subset(&allowed) {
        return Err(rejected(
            422,
            "Assessment references a run outside this job",
        ));
    }
    if body
        .review_versions
        .iter()
        .any(|version| !allowed.contains(version.execution_id.as_str()))
    {
        return Err(rejected(
            422,
            "Review checkpoint references a trace outside this job",
        ));
    }
    let checks = analysis_checks(&job.settings)?;
    let check_ids: BTreeSet<_> = checks.iter().map(|check| check.id.as_str()).collect();
    if body.assessments.iter().any(|assessment| {
        assessment
            .issue_checks
            .iter()
            .chain(&assessment.pattern_checks)
            .any(|id| !check_ids.contains(id.as_str()))
    }) {
        return Err(rejected(422, "Assessment references an unknown check"));
    }
    if body.findings.iter().any(|finding| {
        !check_ids.contains(finding.check_id.as_str())
            || finding
                .check_ids
                .iter()
                .any(|id| !check_ids.contains(id.as_str()))
            || finding
                .evidence
                .iter()
                .any(|evidence| !allowed.contains(evidence.execution_id.as_str()))
    }) {
        return Err(rejected(422, "Finding references evidence outside the job"));
    }
    Ok(())
}

fn finish(
    lens: &Lens,
    job: &Job,
    body: &InvestigationResult,
    historical: &[FindingRun],
    now: DateTime<Utc>,
) -> Result<Lens, Error> {
    let restored = Lens {
        findings: lens
            .findings
            .iter()
            .map(|finding| {
                if !finding.investigation_runs.is_empty() {
                    return finding.clone();
                }
                let runs: BTreeSet<_> = historical
                    .iter()
                    .filter(|run| run.finding_id == finding.id)
                    .map(|run| run.job_id.clone())
                    .collect();
                crate::wire::Finding {
                    investigation_runs: runs.into_iter().collect(),
                    ..finding.clone()
                }
            })
            .collect(),
        ..lens.clone()
    };
    let mut merged = restored.clone();
    for draft in &body.findings {
        let finding = merge_finding(
            &merged,
            draft,
            job.revision,
            now,
            &uuid::Uuid::new_v4().to_string(),
            Some(&job.id),
            false,
        )?;
        merged.findings = std::iter::once(finding.clone())
            .chain(merged.findings.into_iter().filter(|old| {
                old.id != finding.id && !finding.merged_finding_ids.contains(&old.id)
            }))
            .collect();
    }
    let allowed: BTreeSet<_> = job
        .sample
        .iter()
        .flat_map(|sample| &sample.executions)
        .map(|execution| execution.id.as_str())
        .collect();
    let previous = restored
        .findings
        .iter()
        .map(serde_json::to_value)
        .collect::<Result<Vec<_>, _>>()?;
    let findings = merged
        .findings
        .iter()
        .filter_map(|finding| {
            let serialized = match serde_json::to_value(finding) {
                Ok(value) => value,
                Err(error) => return Some(Err(Error::from(error))),
            };
            if previous.contains(&serialized)
                || !finding
                    .evidence
                    .iter()
                    .any(|evidence| allowed.contains(evidence.execution_id.as_str()))
            {
                return None;
            }
            Some(Ok(crate::wire::Finding {
                evidence: finding
                    .evidence
                    .iter()
                    .filter(|evidence| allowed.contains(evidence.execution_id.as_str()))
                    .cloned()
                    .collect(),
                occurrences: finding
                    .occurrences
                    .iter()
                    .filter(|id| allowed.contains(id.as_str()))
                    .cloned()
                    .collect(),
                ..finding.clone()
            }))
        })
        .collect::<Result<Vec<_>, Error>>()?;
    let completed = Job {
        coverage: if !body.error.is_empty()
            && serde_json::to_value(&body.coverage)? == serde_json::to_value(Coverage::default())?
        {
            job.coverage.clone()
        } else {
            body.coverage.clone()
        },
        error: body.error.clone(),
        assessments: body.assessments.clone(),
        review_versions: body.review_versions.clone(),
        findings: Some(findings),
        ..end_job(job, result_status(body), now)
    };
    let interval = TimeDelta::try_minutes(
        lens.settings
            .interval_minutes
            .get()
            .try_into()
            .map_err(|_| Error::InvalidRequest)?,
    )
    .ok_or(Error::InvalidRequest)?;
    Ok(Lens {
        findings: merged.findings,
        last_scan_at: next_scan_start(lens, job, !body.error.is_empty())?,
        next_run_at: now
            .checked_add_signed(interval)
            .ok_or(Error::InvalidRequest)?,
        ..replace_job(lens, completed)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local::fixtures::{StoredJob, stored_job};
    use lens_contract::worker::{Finding, FindingDraft, JobTrigger, Progress, Review};
    use lens_investigations::LensRepository;
    use litellm_storage_clickhouse::execute_statement;
    use rstest::{fixture, rstest};
    use serde_json::{Value, json};
    use std::time::Duration;

    fn decode<T: serde::de::DeserializeOwned>(value: Value) -> T {
        serde_json::from_value(value).unwrap()
    }

    #[fixture]
    fn now() -> DateTime<Utc> {
        "2026-01-01T01:00:00Z".parse().unwrap()
    }

    #[fixture]
    fn job() -> Job {
        let claim: crate::wire::Claim =
            serde_json::from_str(include_str!("../../tests/fixtures/claim.json")).unwrap();
        Job {
            status: JobStatus::Running,
            sample: Some(
                serde_json::from_str(include_str!("../../tests/fixtures/sample.json")).unwrap(),
            ),
            end: now(),
            coverage: Coverage {
                screened: 7,
                ..Default::default()
            },
            worker_id: Some("worker".into()),
            attempts: 1,
            ..claim.job
        }
    }

    #[fixture]
    fn draft() -> FindingDraft {
        decode(
            json!({"title":"Refund outcome contradicts the lookup", "description":"The tool reported failure but the response claimed completion", "check_id":"refund", "kind":"issue",
            "evidence":[{"execution_id":"run-test","span_id":"span-test","quote":"Refund failed","role":"support"}]}),
        )
    }

    #[fixture]
    fn result(draft: FindingDraft) -> InvestigationResult {
        InvestigationResult {
            coverage: Coverage {
                eligible: 1,
                selected: 1,
                screened: 1,
                ..Default::default()
            },
            findings: vec![draft],
            assessments: vec![decode(
                json!({"execution_id":"run-test", "issue_checks":["refund"], "pattern_checks":[], "cannot_assess":false}),
            )],
            review_versions: vec![decode(
                json!({"execution_id":"run-test","content_version":"version"}),
            )],
            error: String::new(),
        }
    }

    #[fixture]
    fn lens(job: Job) -> Lens {
        decode(
            json!({"id":"lens-test", "scope":{"team_id":"team-test"}, "settings":job.settings,
            "created_at":job.created_at, "next_run_at":job.created_at, "budget_month":"2026-01", "jobs":[job]}),
        )
    }

    #[rstest]
    #[case::same_owner(None, 0, false, true)]
    #[case::other_worker(Some("other-worker"), 0, false, false)]
    #[case::other_attempt(None, 1, false, false)]
    #[case::other_job(None, 0, true, false)]
    #[tokio::test]
    async fn completed_result_retries_require_the_same_job_worker_and_attempt(
        #[future(awt)] stored_job: StoredJob,
        #[case] worker: Option<&str>,
        #[case] attempt_delta: i64,
        #[case] other_job: bool,
        #[case] accepted: bool,
    ) {
        let body = InvestigationResult {
            coverage: Coverage::default(),
            findings: vec![],
            assessments: vec![],
            review_versions: vec![],
            error: String::new(),
        };
        stored_job.job.save_result(&body).await.unwrap();
        let completed = stored_job.job.lens().await.unwrap();
        assert_eq!(completed.jobs[0].status, JobStatus::Completed);
        let mut changed = completed.clone();
        if let Some(worker) = worker {
            changed.jobs[0].worker_id = Some(worker.into());
        }
        changed.jobs[0].attempts += attempt_delta;
        let persisted = stored_job
            .job
            .control
            .repository
            .replace(&completed, &changed)
            .await
            .unwrap();
        let mut caller = stored_job.job.clone();
        if other_job {
            caller.job_id = "unrelated-job".into();
        }
        let conflicting = InvestigationResult {
            error: "A retry must not overwrite a saved result".into(),
            ..body
        };
        let result = caller.save_result(&conflicting).await;
        if accepted {
            result.unwrap();
        } else {
            assert!(
                matches!(result, Err(Error::Control { status: 409, .. })),
                "{result:?}"
            );
        }
        let after = stored_job.job.lens().await.unwrap();
        assert_eq!(
            serde_json::to_value(after).unwrap(),
            serde_json::to_value(persisted).unwrap()
        );
    }

    #[rstest]
    #[case::completed("", JobStatus::Completed, true)]
    #[case::failed("Analysis failed", JobStatus::Failed, false)]
    #[tokio::test]
    async fn result_retries_consolidate_only_the_saved_successful_review(
        #[future(awt)] stored_job: StoredJob,
        #[case] error: &str,
        #[case] status: JobStatus,
        #[case] consolidated: bool,
    ) {
        let caller = &stored_job.job;
        let repository = &caller.control.repository;
        let lens = caller.lens().await.unwrap();
        let assigned = Job {
            sample: Some(
                serde_json::from_str(include_str!("../../tests/fixtures/sample.json")).unwrap(),
            ),
            ..lens.jobs[0].clone()
        };
        repository
            .replace(&lens, &replace_job(&lens, assigned.clone()))
            .await
            .unwrap();
        let review: Review = decode(json!({
            "execution_id":"run-test", "trace_id":"trace-test", "agent":"Refund agent",
            "name":"Refund review", "model":"test-analysis", "duration_ms":1,
            "at":Utc::now(), "content_version":"current-content", "extraction":{}
        }));
        repository
            .progress(
                &lens.id,
                &assigned,
                &Progress {
                    review: Some(review.clone()),
                    ..Progress::default()
                },
                Utc::now(),
            )
            .await
            .unwrap()
            .unwrap();
        let storage = &caller.control.sources.0.storage;
        execute_statement(
            &storage.client,
            storage.config.storage().writer(),
            &format!(
                "ALTER TABLE `{}`.lens_state_blobs ADD CONSTRAINT reject_review_completion CHECK NOT \
                 (startsWith(key, 'review/') AND JSONExtractBool(data, 'consolidated'))",
                storage.config.storage().database()
            ),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        let body = InvestigationResult {
            coverage: Coverage::default(),
            findings: vec![],
            assessments: vec![],
            review_versions: vec![crate::wire::ReviewVersion {
                execution_id: review.execution_id.clone(),
                content_version: review.content_version.clone(),
            }],
            error: error.into(),
        };
        let first = caller.save_result(&body).await;
        if consolidated {
            assert!(
                matches!(first, Err(Error::InvestigationStorage(_))),
                "{first:?}"
            );
        } else {
            first.unwrap();
        }
        let saved = caller.lens().await.unwrap();
        assert_eq!(saved.jobs[0].status, status);
        assert_eq!(saved.jobs[0].error, error);
        assert_eq!(
            json!(saved.jobs[0].review_versions),
            json!(body.review_versions)
        );
        assert_eq!(
            json!(repository.reviews(&lens.id, &assigned).await.unwrap()),
            json!([review.clone()])
        );
        execute_statement(
            &storage.client,
            storage.config.storage().writer(),
            &format!(
                "ALTER TABLE `{}`.lens_state_blobs DROP CONSTRAINT reject_review_completion",
                storage.config.storage().database()
            ),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        let retry = InvestigationResult {
            review_versions: vec![],
            error: "Retry payload must not replace the saved result".into(),
            ..body
        };
        caller.save_result(&retry).await.unwrap();
        assert_eq!(json!(caller.lens().await.unwrap()), json!(saved));
        assert_eq!(
            json!(repository.reviews(&lens.id, &assigned).await.unwrap()),
            json!([Review {
                consolidated,
                ..review
            }])
        );
    }

    #[rstest]
    #[case::recorded_quote("Refund failed", true)]
    #[case::invented_quote("Refund completed", false)]
    #[tokio::test]
    async fn finding_evidence_is_verified_against_its_sampled_execution(
        #[future(awt)] stored_job: StoredJob,
        #[case] quote: &str,
        #[case] accepted: bool,
    ) {
        let caller = &stored_job.job;
        let state = &caller.control.sources.0;
        let storage = &state.storage;
        storage.ensure_schema().await.unwrap();
        state
            .schema_ready
            .store(true, std::sync::atomic::Ordering::Release);
        litellm_traces_clickhouse::insert_rows(
            &storage.client,
            storage.config.storage().writer(),
            storage.config.storage().database(),
            litellm_traces_clickhouse::InsertTable::OtelTraces,
            vec![decode(json!({
                "Timestamp":Utc::now().timestamp_nanos_opt().unwrap(), "TraceId":"trace-test",
                "SpanId":"span-test", "TeamId":"team-test", "Output":"Refund failed"
            }))],
        )
        .await
        .unwrap();
        let lens = caller.lens().await.unwrap();
        let assigned = Job {
            sample: Some(
                serde_json::from_str(include_str!("../../tests/fixtures/sample.json")).unwrap(),
            ),
            ..lens.jobs[0].clone()
        };
        let saved = caller
            .control
            .repository
            .replace(&lens, &replace_job(&lens, assigned))
            .await
            .unwrap();
        let finding: FindingDraft = decode(json!({
            "title":"Refund outcome", "description":"Verify the recorded refund result",
            "check_id":"correct", "kind":"issue",
            "evidence":[{"execution_id":"run-test", "span_id":"span-test", "quote":quote}]
        }));
        let body = InvestigationResult {
            coverage: Coverage {
                eligible: 1,
                selected: 1,
                screened: 1,
                ..Coverage::default()
            },
            findings: vec![finding.clone()],
            assessments: vec![],
            review_versions: vec![],
            error: String::new(),
        };
        let result = caller.save_result(&body).await;
        let after = caller.lens().await.unwrap();
        if accepted {
            result.unwrap();
            assert_eq!(after.jobs[0].status, JobStatus::Completed);
            assert_eq!(after.findings.len(), 1);
            assert_eq!(json!(after.findings[0].evidence), json!(finding.evidence));
        } else {
            assert!(
                matches!(result, Err(Error::Control { status: 422, .. })),
                "{result:?}"
            );
            assert_eq!(json!(after), json!(saved));
        }
    }

    #[rstest]
    #[case::existing_same_kind(false, "existing", "issue", true)]
    #[case::existing_missing(false, "missing", "issue", false)]
    #[case::existing_other_kind(false, "existing", "pattern", false)]
    #[case::merged_same_kind(true, "existing", "issue", true)]
    #[case::merged_missing(true, "missing", "issue", false)]
    #[case::merged_other_kind(true, "existing", "pattern", false)]
    #[tokio::test]
    async fn finding_references_require_this_investigation_and_kind(
        #[future(awt)] stored_job: StoredJob,
        #[case] merged: bool,
        #[case] referenced: &str,
        #[case] previous_kind: &str,
        #[case] accepted: bool,
    ) {
        let lens = stored_job.job.lens().await.unwrap();
        let previous: FindingDraft = decode(
            json!({"title":"Prior finding", "description":"Prior evidence", "check_id":"correct", "kind":previous_kind, "evidence":[]}),
        );
        let finding =
            merge_finding(&lens, &previous, 1, Utc::now(), "existing", None, false).unwrap();
        let with_finding = Lens {
            findings: vec![finding],
            ..lens.clone()
        };
        let persisted = stored_job
            .job
            .control
            .repository
            .replace(&lens, &with_finding)
            .await
            .unwrap();
        let draft: FindingDraft = decode(
            json!({"title":"Updated finding", "description":"Current evidence", "check_id":"correct", "kind":"issue", "evidence":[],
            "existing_finding_id": (!merged).then_some(referenced), "merged_finding_ids": if merged { vec![referenced] } else { vec![] }}),
        );
        let body = InvestigationResult {
            coverage: Coverage::default(),
            findings: vec![draft],
            assessments: vec![],
            review_versions: vec![],
            error: String::new(),
        };
        let result = stored_job.job.save_result(&body).await;
        let after = stored_job.job.lens().await.unwrap();
        if accepted {
            result.unwrap();
            assert_eq!(after.jobs[0].status, JobStatus::Completed);
            assert_eq!(after.findings.len(), 1);
            assert_eq!(after.findings[0].id, "existing");
            assert_eq!(after.findings[0].title.as_str(), "Updated finding");
        } else {
            assert!(
                matches!(result, Err(Error::Control { status: 422, .. })),
                "{result:?}"
            );
            assert_eq!(
                serde_json::to_value(after).unwrap(),
                serde_json::to_value(persisted).unwrap()
            );
        }
    }

    #[rstest]
    #[case::duplicate_assessment("/assessments", json!([
        {"execution_id":"run-test","issue_checks":[],"pattern_checks":[],"cannot_assess":false},
        {"execution_id":"run-test","issue_checks":[],"pattern_checks":[],"cannot_assess":false}
    ]))]
    #[case::foreign_assessment("/assessments/0/execution_id", json!("other"))]
    #[case::foreign_checkpoint("/review_versions/0/execution_id", json!("other"))]
    #[case::unknown_issue_check("/assessments/0/issue_checks", json!(["other"]))]
    #[case::unknown_pattern_check("/assessments/0/pattern_checks", json!(["other"]))]
    #[case::unknown_finding_check("/findings/0/check_id", json!("other"))]
    #[case::unknown_merged_check("/findings/0/check_ids", json!(["other"]))]
    #[case::foreign_evidence("/findings/0/evidence/0/execution_id", json!("other"))]
    fn result_rejects_references_outside_the_frozen_sample_or_checks(
        job: Job,
        result: InvestigationResult,
        #[case] pointer: &str,
        #[case] replacement: Value,
    ) {
        let mut body = serde_json::to_value(result).unwrap();
        let (parent, field) = pointer.rsplit_once('/').unwrap();
        body.pointer_mut(parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert(field.into(), replacement);
        let error = validate(&job, job.sample.as_ref().unwrap(), &decode(body)).unwrap_err();
        assert!(matches!(error, Error::Control { status: 422, .. }));
    }

    #[rstest]
    fn valid_result_keeps_assessments_and_checkpoints(job: Job, result: InvestigationResult) {
        validate(&job, job.sample.as_ref().unwrap(), &result).unwrap();
    }

    #[rstest]
    #[case::complete("", false, (JobStatus::Completed, 1))]
    #[case::empty_failure("storage unavailable", true, (JobStatus::Failed, 7))]
    #[case::partial_failure("one trace unavailable", false, (JobStatus::Completed, 1))]
    fn completed_run_preserves_progress_and_schedules_from_its_result(
        lens: Lens,
        job: Job,
        result: InvestigationResult,
        now: DateTime<Utc>,
        #[case] error: &str,
        #[case] empty_coverage: bool,
        #[case] expected: (JobStatus, i64),
    ) {
        let body = InvestigationResult {
            error: error.into(),
            coverage: if empty_coverage {
                Coverage::default()
            } else {
                result.coverage.clone()
            },
            findings: if empty_coverage {
                vec![]
            } else {
                result.findings.clone()
            },
            assessments: if empty_coverage {
                vec![]
            } else {
                result.assessments.clone()
            },
            ..result
        };
        let saved = finish(&lens, &job, &body, &[], now).unwrap();
        let finished = &saved.jobs[0];
        assert_eq!(finished.status, expected.0);
        assert_eq!(finished.finished_at, Some(now));
        assert_eq!(finished.error, error);
        assert_eq!(finished.coverage.screened, expected.1);
        assert_eq!(json!(finished.assessments), json!(body.assessments));
        assert_eq!(json!(finished.review_versions), json!(body.review_versions));
        assert_eq!(saved.next_run_at, now + TimeDelta::minutes(15));
        assert_eq!(
            saved.last_scan_at,
            if error.is_empty() {
                Some(job.end)
            } else {
                None
            }
        );
        assert_eq!(saved.findings.len(), body.findings.len());
        assert_eq!(
            finished.findings.as_ref().unwrap().len(),
            body.findings.len()
        );
        if !body.findings.is_empty() {
            assert_eq!(saved.findings[0].investigation_runs, vec![job.id]);
        }
    }

    #[rstest]
    fn manual_completion_does_not_advance_the_scheduled_scan(
        lens: Lens,
        job: Job,
        result: InvestigationResult,
        now: DateTime<Utc>,
    ) {
        let manual = Job {
            trigger: JobTrigger::Manual,
            ..job
        };
        let saved = finish(&lens, &manual, &result, &[], now).unwrap();
        assert_eq!(saved.last_scan_at, lens.last_scan_at);
        assert_eq!(saved.jobs[0].status, JobStatus::Completed);
    }

    #[rstest]
    fn repeated_finding_restores_history_but_run_snapshot_only_contains_current_evidence(
        lens: Lens,
        job: Job,
        draft: FindingDraft,
        now: DateTime<Utc>,
        result: InvestigationResult,
    ) {
        let earlier = Finding {
            evidence: vec![decode(
                json!({"execution_id":"prior-run", "span_id":"prior-span","quote":"Prior refund failed", "role":"support"}),
            )],
            occurrences: vec!["prior-run".into()],
            investigation_runs: vec![],
            ..merge_finding(
                &lens,
                &draft,
                1,
                now - TimeDelta::hours(1),
                "existing",
                None,
                false,
            )
            .unwrap()
        };
        let existing = Lens {
            findings: vec![earlier],
            ..lens
        };
        let historical = vec![FindingRun {
            finding_id: "existing".into(),
            job_id: "archived".into(),
        }];
        let body = InvestigationResult {
            findings: vec![FindingDraft {
                existing_finding_id: Some("existing".into()),
                ..draft
            }],
            ..result
        };
        let saved = finish(&existing, &job, &body, &historical, now).unwrap();
        assert_eq!(saved.findings.len(), 1);
        let finding = &saved.findings[0];
        assert_eq!(finding.id, "existing");
        assert_eq!(finding.evidence.len(), 2);
        assert!(finding.investigation_runs.contains(&"archived".into()));
        assert!(finding.investigation_runs.contains(&job.id));
        let snapshot = &saved.jobs[0].findings.as_ref().unwrap()[0];
        assert_eq!(snapshot.evidence.len(), 1);
        assert_eq!(snapshot.evidence[0].execution_id, "run-test");
        assert_eq!(snapshot.occurrences, vec!["run-test"]);
    }
}
