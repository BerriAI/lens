use crate::{State, wait_for_read_slot};
use chrono::{DateTime, SecondsFormat, Utc};
use lens_contract::{
    feedback::{Feedback, TraceFeedback, TraceFeedbackSummary, TraceIdentity},
    investigations::Scope,
};
use lens_server::feedback::{FeedbackStore, FeedbackStoreError, FeedbackWrite};
use litellm_storage_clickhouse::fetch;
use litellm_traces_clickhouse::{
    InsertTable, insert_rows,
    query::lens::{
        LensAccessParams, LensFeedback, LensFeedbackParams, LensFeedbackRow, LensFeedbackSummary,
        LensFeedbackSummaryParams, LensFeedbackTarget, LensFeedbackTargetParams,
        LensFeedbackTargetRow,
    },
};
use serde_json::json;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::OwnedSemaphorePermit;

pub struct FeedbackApi(pub Arc<State>);

fn failure(error: impl std::error::Error + Send + Sync + 'static) -> FeedbackStoreError {
    FeedbackStoreError(Box::new(error))
}
fn invalid_row() -> FeedbackStoreError {
    failure(litellm_traces_clickhouse::Error::InvalidRow)
}
fn access(scope: &Scope) -> LensAccessParams {
    LensAccessParams {
        all_teams: scope.all_teams,
        team: scope.team_id.clone(),
        key_hash: scope.api_key_hash.clone(),
    }
}

fn feedback(row: LensFeedbackRow) -> Result<Feedback, FeedbackStoreError> {
    let score = u8::try_from(row.score)
        .ok()
        .filter(|score| *score <= 10)
        .ok_or_else(invalid_row)?;
    Ok(Feedback {
        trace_id: row.trace_id,
        trace_ref: row.trace_ref,
        score,
        comment: row.comment,
        author: row.author,
        created_at: DateTime::parse_from_rfc3339(&row.created_at)
            .map_err(failure)?
            .with_timezone(&Utc),
        updated_at: DateTime::parse_from_rfc3339(&row.updated_at)
            .map_err(failure)?
            .with_timezone(&Utc),
    })
}

impl FeedbackApi {
    async fn permit(&self) -> Result<OwnedSemaphorePermit, FeedbackStoreError> {
        self.0.require_storage().map_err(failure)?;
        wait_for_read_slot(self.0.read_slots.clone().acquire_owned())
            .await
            .map_err(failure)
    }

    async fn target(
        &self,
        scope: &Scope,
        trace: &TraceIdentity,
    ) -> Result<Option<LensFeedbackTargetRow>, FeedbackStoreError> {
        let rows = fetch::<LensFeedbackTarget>(
            &self.0.storage.client,
            self.0.storage.config.storage().reader(),
            &LensFeedbackTargetParams {
                access: access(scope),
                trace_id: trace.trace_id.clone(),
                trace_ref: trace.trace_ref.clone(),
            },
        )
        .await
        .map_err(failure)?;
        if rows.len() != 1 {
            return Ok(None);
        }
        Ok(rows.into_iter().next())
    }

    async fn rows(
        &self,
        scope: &Scope,
        trace: &TraceIdentity,
    ) -> Result<Vec<Feedback>, FeedbackStoreError> {
        fetch::<LensFeedback>(
            &self.0.storage.client,
            self.0.storage.config.storage().reader(),
            &LensFeedbackParams {
                access: access(scope),
                trace_id: trace.trace_id.clone(),
                trace_ref: trace.trace_ref.clone(),
            },
        )
        .await
        .map_err(failure)?
        .into_iter()
        .map(feedback)
        .collect()
    }

    async fn write(
        &self,
        target: LensFeedbackTargetRow,
        feedback: &Feedback,
        deleted: bool,
    ) -> Result<(), FeedbackStoreError> {
        let row = BTreeMap::from([
            ("TeamId".into(), json!(target.team_id)),
            ("ApiKeyHash".into(), json!(target.key_hash)),
            ("TraceId".into(), json!(feedback.trace_id)),
            ("Author".into(), json!(feedback.author)),
            ("Score".into(), json!(feedback.score)),
            ("Comment".into(), json!(feedback.comment)),
            (
                "CreatedAt".into(),
                json!(
                    feedback
                        .created_at
                        .to_rfc3339_opts(SecondsFormat::Millis, true)
                ),
            ),
            (
                "UpdatedAt".into(),
                json!(
                    feedback
                        .updated_at
                        .to_rfc3339_opts(SecondsFormat::Millis, true)
                ),
            ),
            ("IsDeleted".into(), json!(u8::from(deleted))),
        ]);
        insert_rows(
            &self.0.storage.client,
            self.0.storage.config.storage().writer(),
            self.0.storage.config.storage().database(),
            InsertTable::LensFeedback,
            vec![row],
        )
        .await
        .map_err(failure)
    }
}

impl FeedbackStore for FeedbackApi {
    async fn for_trace(
        &self,
        scope: &Scope,
        trace: &TraceIdentity,
    ) -> Result<Option<TraceFeedback>, FeedbackStoreError> {
        let _permit = self.permit().await?;
        let Some(target) = self.target(scope, trace).await? else {
            return Ok(None);
        };
        let trace = TraceIdentity {
            trace_id: trace.trace_id.clone(),
            trace_ref: target.trace_ref,
        };
        let feedback = self.rows(scope, &trace).await?;
        Ok(Some(TraceFeedback {
            trace_id: trace.trace_id,
            trace_ref: trace.trace_ref,
            feedback,
        }))
    }

    async fn upsert(
        &self,
        scope: &Scope,
        write: &FeedbackWrite,
    ) -> Result<Option<Feedback>, FeedbackStoreError> {
        let _permit = self.permit().await?;
        let Some(target) = self.target(scope, &write.trace).await? else {
            return Ok(None);
        };
        let trace = TraceIdentity {
            trace_id: write.trace.trace_id.clone(),
            trace_ref: target.trace_ref.clone(),
        };
        let previous = self
            .rows(scope, &trace)
            .await?
            .into_iter()
            .find(|feedback| feedback.author == write.author);
        let feedback = Feedback {
            trace_id: trace.trace_id,
            trace_ref: trace.trace_ref,
            score: write.score,
            comment: write.comment.clone(),
            author: write.author.clone(),
            created_at: previous
                .map(|feedback| feedback.created_at)
                .unwrap_or(write.at),
            updated_at: write.at,
        };
        self.write(target, &feedback, false).await?;
        Ok(Some(feedback))
    }

    async fn delete(
        &self,
        scope: &Scope,
        trace: &TraceIdentity,
        author: &str,
        at: DateTime<Utc>,
    ) -> Result<Option<bool>, FeedbackStoreError> {
        let _permit = self.permit().await?;
        let Some(target) = self.target(scope, trace).await? else {
            return Ok(None);
        };
        let trace = TraceIdentity {
            trace_id: trace.trace_id.clone(),
            trace_ref: target.trace_ref.clone(),
        };
        let Some(previous) = self
            .rows(scope, &trace)
            .await?
            .into_iter()
            .find(|feedback| feedback.author == author)
        else {
            return Ok(Some(false));
        };
        self.write(
            target,
            &Feedback {
                comment: String::new(),
                updated_at: at,
                ..previous
            },
            true,
        )
        .await?;
        Ok(Some(true))
    }

    async fn summaries(
        &self,
        scope: &Scope,
        traces: &[TraceIdentity],
    ) -> Result<Vec<TraceFeedbackSummary>, FeedbackStoreError> {
        let _permit = self.permit().await?;
        let trace_ids = traces
            .iter()
            .map(|trace| trace.trace_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let rows = fetch::<LensFeedbackSummary>(
            &self.0.storage.client,
            self.0.storage.config.storage().reader(),
            &LensFeedbackSummaryParams {
                access: access(scope),
                trace_ids,
            },
        )
        .await
        .map_err(failure)?;
        let mut summaries = Vec::new();
        for trace in traces {
            let matching: Vec<_> = rows
                .iter()
                .filter(|row| {
                    row.trace_id == trace.trace_id
                        && (trace.trace_ref.is_empty() || trace.trace_ref == row.trace_ref)
                })
                .collect();
            if matching.is_empty() {
                summaries.push(TraceFeedbackSummary {
                    trace_id: trace.trace_id.clone(),
                    trace_ref: trace.trace_ref.clone(),
                    count: 0,
                    average: None,
                    lowest: None,
                });
                continue;
            }
            for row in matching {
                let lowest = u8::try_from(row.lowest)
                    .ok()
                    .filter(|value| *value <= 10)
                    .ok_or_else(invalid_row)?;
                summaries.push(TraceFeedbackSummary {
                    trace_id: row.trace_id.clone(),
                    trace_ref: row.trace_ref.clone(),
                    count: row.count,
                    average: Some(row.average),
                    lowest: Some(lowest),
                });
            }
        }
        Ok(summaries)
    }
}
