use crate::worker::{
    Activity, Claim, Execution, ExecutionContent, Extraction, Finding, FindingDraft, Job,
    LensSettings, Observation, Progress, Result as WorkerResult, Review, RunAssessment, Sample,
};
use serde::{Serialize, Serializer, ser::SerializeStruct};

#[derive(Clone, Debug, serde::Deserialize, schemars::JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct Public<T>(pub T);

pub trait PublicRecord {
    fn serialize_public<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error>;
}

impl<T: PublicRecord> Serialize for Public<T> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize_public(serializer)
    }
}

impl<T: PublicRecord + ?Sized> PublicRecord for &T {
    fn serialize_public<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        T::serialize_public(self, serializer)
    }
}

impl<T: PublicRecord> PublicRecord for Vec<T> {
    fn serialize_public<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(self.iter().map(Public))
    }
}

impl<T: PublicRecord> PublicRecord for Option<T> {
    fn serialize_public<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Some(value) => serializer.serialize_some(&Public(value)),
            None => serializer.serialize_none(),
        }
    }
}

pub fn serialize<T: PublicRecord, S: Serializer>(
    value: &T,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.serialize_public(serializer)
}

macro_rules! public_field {
    ($state:ident, $field:ident) => {
        $state.serialize_field(stringify!($field), $field)?;
    };
    ($state:ident, $field:ident, nested) => {
        $state.serialize_field(stringify!($field), &Public($field))?;
    };
}

macro_rules! public_record {
    ($record:ident { $($field:ident $(=> $nested:ident)?),* $(,)? }) => {
        impl PublicRecord for $record {
            fn serialize_public<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let $record { $($field),* } = self;
                let mut state = serializer.serialize_struct(stringify!($record), [$(stringify!($field)),*].len())?;
                $(public_field!(state, $field $(, $nested)?);)*
                state.end()
            }
        }
    };
}

public_record!(LensSettings {
    source,
    service,
    agent_name,
    filters,
    sample_size,
    sample_percent,
    team_id,
    execution_ids,
    name,
    context,
    lookback_hours,
    checks,
    model,
    enabled,
    interval_minutes,
    concurrency,
    monthly_budget,
});

public_record!(Job {
    id, status, stage, created_at, start, end, settings => nested, revision, worker_id, lease_until,
    attempts, finished_at, coverage, error, sample => nested, cost, findings => nested,
    assessments => nested, steps, reviews => nested, reviewed, reading, activities => nested,
    trigger, review_versions,
});

public_record!(Finding {
    title,
    description,
    check_id,
    kind,
    priority,
    suggestion,
    limitation,
    brief,
    evidence,
    existing_finding_id,
    check_ids,
    merged_finding_ids,
    id,
    status,
    reason,
    first_seen,
    last_seen,
    occurrences,
    revision,
    investigation_runs,
});

public_record!(FindingDraft {
    title,
    description,
    check_id,
    kind,
    priority,
    suggestion,
    limitation,
    brief,
    evidence,
    existing_finding_id,
    check_ids,
    merged_finding_ids,
});

public_record!(Sample { executions => nested, eligible, selected, next_offset, next_cursor });

public_record!(Execution {
    id,
    source,
    trace_id,
    trace_ref,
    team_id,
    name,
    start_time,
    span_count,
    root_seen,
    service,
    metadata,
});

public_record!(ExecutionContent { execution => nested, parts, next_cursor, partial });

public_record!(Review {
    execution_id, trace_id, agent, name, spans, reasoning, verdicts, cannot_assess, model, duration_ms,
    at, tool_calls, extraction => nested, content_version, reused, consolidated, partial,
});

public_record!(Extraction { observations => nested, cannot_assess, reasoning });
public_record!(Observation {
    check_id,
    kind,
    summary,
    evidence
});
public_record!(RunAssessment {
    execution_id,
    issue_checks,
    pattern_checks,
    cannot_assess
});

public_record!(Activity {
    id,
    phase,
    label,
    execution_ids,
    started_at,
    operations,
    tool_calls,
    finished,
});

public_record!(Claim { lens_id, job => nested, findings => nested, reviews => nested });
public_record!(Progress { stage, coverage, review => nested, reading, activity => nested });
public_record!(WorkerResult { assessments => nested, findings => nested, coverage, error, review_versions });
