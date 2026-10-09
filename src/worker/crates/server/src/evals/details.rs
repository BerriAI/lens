use lens_contract::{
    datasets::DatasetRole,
    eval::{EvalBaselineDetails, EvalCaseDetails, EvalRunDetails},
};
use lens_evals::StoredRun;

fn cases(record: &StoredRun) -> Vec<EvalCaseDetails> {
    record
        .cases
        .iter()
        .map(|case| EvalCaseDetails {
            case_id: case.id.clone(),
            input: case
                .messages
                .iter()
                .find(|message| message.role == DatasetRole::User)
                .map(|message| message.content.clone())
                .unwrap_or_default(),
            verdict: record.verdicts.get(&case.id).copied(),
            traces: record
                .resolved_traces
                .get(&case.id)
                .cloned()
                .unwrap_or_default(),
        })
        .collect()
}

pub(super) fn details(record: StoredRun, runs: &[StoredRun]) -> EvalRunDetails {
    let baseline = record
        .run
        .summary
        .as_ref()
        .and_then(|summary| summary.baseline_run_id.as_ref())
        .and_then(|id| {
            runs.iter()
                .find(|run| run.run.id == *id && run.team_id == record.team_id)
        })
        .map(|run| EvalBaselineDetails {
            run: run.run.clone(),
            cases: cases(run),
        });
    EvalRunDetails {
        cases: cases(&record),
        run: record.run,
        dataset_id: record.spec.dataset_id,
        dataset_revision: record.spec.revision,
        created_at: record.created_at,
        completed_at: record.completed_at,
        ci_url: record.spec.ci_url,
        baseline,
    }
}
