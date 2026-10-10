use std::sync::Arc;

use lens_contract::{
    activity::ActivitySelection,
    execution::ExecutionId,
    investigations::Scope,
    worker::{
        Evidence, Execution, ExecutionContent, ExecutionSource, LensSettingsSource, MetadataFilter,
        Sample, TracePart,
    },
};
use litellm_storage_clickhouse::{Query, fetch};
use litellm_traces_clickhouse::query::lens::{
    self, ContentSource, LensAccessParams, LensAgents, LensAgentsParams, LensAvailability,
    LensAvailabilityParams, LensAvailabilityRow, LensContent, LensContentParams, LensContentRow,
    LensEvidence, LensEvidenceParams, LensSample, LensSampleEligibility, LensSampleParams,
    LensSampleRow, LensSignalSample,
};

use crate::{Error, State, wait_for_read_slot};

#[derive(Clone)]
pub struct SourceReader(pub Arc<State>);

pub struct SampleRequest<'a> {
    pub selection: &'a ActivitySelection,
    pub start: u64,
    pub end: u64,
    pub offset: u64,
    pub page_size: u32,
    pub preview: bool,
    pub cursor: &'a str,
}

impl SourceReader {
    async fn read<Q: Query>(&self, parameters: &Q::Params) -> Result<Vec<Q::Row>, Error> {
        self.0.require_storage()?;
        let _permit = wait_for_read_slot(self.0.read_slots.clone().acquire_owned()).await?;
        Ok(fetch::<Q>(
            &self.0.storage.client,
            self.0.storage.config.storage().reader(),
            parameters,
        )
        .await?)
    }

    pub async fn availability(&self, scope: &Scope) -> Result<LensAvailabilityRow, Error> {
        Ok(self
            .read::<LensAvailability>(&LensAvailabilityParams {
                access: access(scope),
            })
            .await?
            .into_iter()
            .next()
            .unwrap_or(LensAvailabilityRow {
                traces: 0,
                requests: 0,
            }))
    }

    pub async fn agents(&self, scope: &Scope) -> Result<Vec<String>, Error> {
        Ok(self
            .read::<LensAgents>(&LensAgentsParams {
                access: access(scope),
            })
            .await?
            .into_iter()
            .map(|row| row.agent_name)
            .collect())
    }

    pub async fn sample(&self, scope: &Scope, request: SampleRequest<'_>) -> Result<Sample, Error> {
        let parameters = sample_parameters(scope, &request)?;
        let rows = self.read::<LensSample>(&parameters).await?;
        sample_page(rows, &request)
    }

    pub(crate) async fn signal_sample(
        &self,
        scope: &Scope,
        request: SampleRequest<'_>,
    ) -> Result<Sample, Error> {
        let parameters = sample_parameters(scope, &request)?;
        let rows = self.read::<LensSignalSample>(&parameters).await?;
        sample_page(rows, &request)
    }

    pub(crate) async fn sample_eligible(
        &self,
        scope: &Scope,
        request: SampleRequest<'_>,
    ) -> Result<u64, Error> {
        let parameters = sample_parameters(scope, &request)?;
        let rows = self.read::<LensSampleEligibility>(&parameters).await?;
        Ok(rows.first().map_or(0, |row| row.eligible))
    }

    pub async fn content(
        &self,
        scope: &Scope,
        execution: &Execution,
        cursor: &str,
        offset: Option<u32>,
    ) -> Result<ExecutionContent, Error> {
        let rows = self
            .read::<LensContent>(&LensContentParams {
                access: access(scope),
                source: source(execution.source),
                id: execution.trace_id.clone(),
                trace_ref: execution.trace_ref.clone(),
                record_team: execution.team_id.clone(),
                start_time: execution.start_time.clone(),
                cursor: cursor.into(),
                offset: match offset {
                    Some(offset) => offset.checked_add(1).ok_or(Error::InvalidRequest)?,
                    None => 0,
                },
            })
            .await?;
        Ok(content_page(execution, rows))
    }

    pub async fn verify_evidence(
        &self,
        scope: &Scope,
        execution: &Execution,
        evidence: &Evidence,
    ) -> Result<bool, Error> {
        let rows = self
            .read::<LensEvidence>(&LensEvidenceParams {
                access: access(scope),
                source: source(execution.source),
                id: execution.trace_id.clone(),
                trace_ref: execution.trace_ref.clone(),
                record_team: execution.team_id.clone(),
                start_time: execution.start_time.clone(),
                span: evidence.span_id.clone(),
                quote: evidence.quote.to_string(),
            })
            .await?;
        Ok(rows.first().is_some_and(|row| row.count != 0))
    }
}

fn access(scope: &Scope) -> LensAccessParams {
    LensAccessParams {
        all_teams: scope.all_teams,
        team: scope.team_id.clone(),
        key_hash: scope.api_key_hash.clone(),
    }
}

fn source(source: ExecutionSource) -> ContentSource {
    match source {
        ExecutionSource::Traces => ContentSource::Traces,
        ExecutionSource::Requests => ContentSource::Requests,
    }
}

fn sample_parameters(
    scope: &Scope,
    request: &SampleRequest<'_>,
) -> Result<LensSampleParams, Error> {
    let settings = request.selection;
    Ok(LensSampleParams {
        access: access(scope),
        source: match settings.source {
            LensSettingsSource::Traces => lens::ExecutionSource::Traces,
            LensSettingsSource::Requests => lens::ExecutionSource::Requests,
            LensSettingsSource::Both => lens::ExecutionSource::Both,
        },
        start: request.start,
        end: request.end,
        agent_name: settings.agent_name.clone(),
        service: settings.service.clone(),
        filter_keys: settings
            .filters
            .iter()
            .map(|filter| filter.key.to_string())
            .collect(),
        filter_values: settings
            .filters
            .iter()
            .map(|filter| filter.value.to_string())
            .collect(),
        selected_team: settings.team_id.clone(),
        execution_ids: settings
            .execution_ids
            .iter()
            .map(|id| {
                ExecutionId::decode(id)
                    .map(|identity| identity.selection_key())
                    .ok_or(Error::InvalidRequest)
            })
            .collect::<Result<_, _>>()?,
        sample_cap: settings.sample_size.map_or(0, |size| size.get()),
        sample_percent: settings.sample_percent,
        preview: u8::from(request.preview),
        after: request.cursor.into(),
        limit: request.page_size,
        offset: request.offset,
    })
}

fn sample_page(rows: Vec<LensSampleRow>, request: &SampleRequest<'_>) -> Result<Sample, Error> {
    let eligible = rows.first().map_or(0, |row| row.eligible);
    let selected = rows.first().map_or(0.0, |row| row.selected);
    if !selected.is_finite()
        || selected < 0.0
        || selected.fract() != 0.0
        || selected >= 9_223_372_036_854_775_808.0
    {
        return Err(Error::Unavailable);
    }
    let selected = selected as i64;
    let next = request
        .offset
        .checked_add(rows.len() as u64)
        .ok_or(Error::Unavailable)?;
    let total = if request.preview {
        eligible
    } else {
        selected as u64
    };
    let next_offset = if request.page_size != 0 && !rows.is_empty() && next < total {
        Some(i64::try_from(next).map_err(|_| Error::Unavailable)?)
    } else {
        None
    };
    let next_cursor = (rows.len() == request.page_size as usize)
        .then(|| rows.last().map(|row| row.selection_key.clone()))
        .flatten();
    let executions = rows.into_iter().map(execution).collect::<Result<_, _>>()?;
    Ok(Sample {
        executions,
        eligible: i64::try_from(eligible).map_err(|_| Error::Unavailable)?,
        selected,
        next_cursor,
        next_offset,
    })
}

fn execution(row: LensSampleRow) -> Result<Execution, Error> {
    let source = match row.source {
        ContentSource::Traces => ExecutionSource::Traces,
        ContentSource::Requests => ExecutionSource::Requests,
    };
    let id = ExecutionId {
        source: source.to_string(),
        team_id: row.team_id.clone(),
        trace_id: row.trace_id.clone(),
        trace_ref: row.trace_ref.clone(),
    }
    .encode();
    let metadata = row
        .attributes
        .into_iter()
        .filter(|(key, value)| {
            key != "litellm.api_key_hash" && !key.is_empty() && !value.is_empty()
        })
        .map(|(key, value)| {
            Ok(MetadataFilter {
                key: key.try_into().map_err(|_| Error::Unavailable)?,
                value: value.try_into().map_err(|_| Error::Unavailable)?,
            })
        })
        .collect::<Result<_, Error>>()?;
    Ok(Execution {
        id,
        source,
        trace_id: row.trace_id,
        trace_ref: row.trace_ref,
        team_id: row.team_id,
        name: row.name,
        start_time: row.start_time,
        span_count: i64::try_from(row.span_count).map_err(|_| Error::Unavailable)?,
        root_seen: row.root_seen != 0,
        service: row.service,
        metadata,
    })
}

fn content_page(execution: &Execution, rows: Vec<LensContentRow>) -> ExecutionContent {
    let next_cursor = (rows.len() == 40)
        .then(|| rows.last().map(|row| row.span_id.clone()))
        .flatten();
    let partial = !execution.root_seen || rows.iter().any(|row| row.truncated != 0);
    let parts = rows
        .into_iter()
        .map(|row| TracePart {
            execution_id: execution.id.clone(),
            span_id: row.span_id,
            parent_span_id: row.parent_span_id,
            name: row.name,
            kind: row.kind,
            start_time: row.start_time,
            end_time: row.end_time,
            content: row.content,
            truncated: row.truncated != 0,
        })
        .collect();
    ExecutionContent {
        execution: execution.clone(),
        parts,
        next_cursor,
        partial,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::{fixture, rstest};
    use serde_json::json;

    #[fixture]
    fn row() -> LensSampleRow {
        serde_json::from_value(json!({"source":"traces","trace_id":"trace","team_id":"team","trace_ref":"ref","name":"agent","start_time":"2026-01-01 00:00:00","span_count":1,"root_seen":1,"service":"service","attributes":[["litellm.api_key_hash","private-hash"],["version","v1"],["","empty-key"],["empty-value",""]],"eligible":3,"position":1,"selected":2,"selection_key":"cursor"})).unwrap()
    }

    #[rstest]
    #[case::selected_first(false, 0, 1, Some(1), Some("cursor"))]
    #[case::selected_last(false, 1, 1, None, Some("cursor"))]
    #[case::preview_more(true, 1, 1, Some(2), Some("cursor"))]
    #[case::short_page(false, 0, 100, Some(1), None)]
    #[case::zero_page(false, 0, 0, None, None)]
    fn pagination_preserves_selection_and_preview_counts(
        row: LensSampleRow,
        #[case] preview: bool,
        #[case] offset: u64,
        #[case] page_size: u32,
        #[case] next_offset: Option<i64>,
        #[case] next_cursor: Option<&str>,
    ) {
        let selection = ActivitySelection::default();
        let page = sample_page(
            vec![row],
            &SampleRequest {
                selection: &selection,
                start: 0,
                end: 1,
                offset,
                page_size,
                preview,
                cursor: "",
            },
        )
        .unwrap();
        assert_eq!(page.eligible, 3);
        assert_eq!(page.selected, 2);
        assert_eq!(page.next_offset, next_offset);
        assert_eq!(page.next_cursor.as_deref(), next_cursor);
        assert_eq!(page.executions[0].metadata.len(), 1);
        assert_eq!(page.executions[0].metadata[0].key.as_str(), "version");
        assert_eq!(page.executions[0].metadata[0].value.as_str(), "v1");
        assert_eq!(
            ExecutionId::decode(&page.executions[0].id)
                .unwrap()
                .selection_key(),
            "traces\0team\0ref"
        );
    }

    #[rstest]
    #[case::negative(-1.0)]
    #[case::fractional(0.5)]
    #[case::nan(f64::NAN)]
    #[case::infinite(f64::INFINITY)]
    #[case::overflow(9_223_372_036_854_775_808.0)]
    fn invalid_database_count_is_not_truncated(row: LensSampleRow, #[case] selected: f64) {
        let selection = ActivitySelection::default();
        assert!(
            sample_page(
                vec![LensSampleRow { selected, ..row }],
                &SampleRequest {
                    selection: &selection,
                    start: 0,
                    end: 1,
                    offset: 0,
                    page_size: 100,
                    preview: false,
                    cursor: ""
                }
            )
            .is_err()
        );
    }

    #[rstest]
    fn sampling_keeps_all_access_and_selection_constraints() {
        let execution = ExecutionId {
            source: "traces".into(),
            team_id: "team".into(),
            trace_id: "trace".into(),
            trace_ref: "ref".into(),
        }
        .encode();
        let selection:ActivitySelection=serde_json::from_value(json!({"source":"both","service":"service","agent_name":"agent","filters":[{"key":"version","value":"v1"}],"sample_size":5,"sample_percent":25,"team_id":"selected-team","execution_ids":[execution]})).unwrap();
        let scope = Scope {
            all_teams: false,
            team_id: "scope-team".into(),
            api_key_hash: "scope-key".into(),
        };
        let parameters = sample_parameters(
            &scope,
            &SampleRequest {
                selection: &selection,
                start: 10,
                end: 20,
                offset: 30,
                page_size: 40,
                preview: true,
                cursor: "after",
            },
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(parameters).unwrap(),
            json!({"all_teams":0,"team":"scope-team","key_hash":"scope-key","source":"both","start":10,"end":20,"agent_name":"agent","service":"service","filter_keys":["version"],"filter_values":["v1"],"selected_team":"selected-team","execution_ids":["traces\0team\0ref"],"sample_cap":5,"sample_percent":25.0,"preview":1,"after":"after","limit":40,"offset":30})
        );
    }

    #[rstest]
    #[case::complete(true, false, false)]
    #[case::root_missing(false, false, true)]
    #[case::truncated(true, true, true)]
    fn content_marks_incomplete_reads(
        row: LensSampleRow,
        #[case] root_seen: bool,
        #[case] truncated: bool,
        #[case] partial: bool,
    ) {
        let execution = Execution {
            root_seen,
            ..execution(row).unwrap()
        };
        let row:LensContentRow=serde_json::from_value(json!({"span_id":"span","parent_span_id":"parent","name":"tool","kind":"tool","start_time":"start","end_time":"end","content":"quote","truncated":u8::from(truncated)})).unwrap();
        let content = content_page(&execution, vec![row]);
        assert_eq!(content.partial, partial);
        assert_eq!(content.parts[0].execution_id, execution.id);
        assert_eq!(content.parts[0].parent_span_id, "parent");
        assert_eq!(content.parts[0].content, "quote");
        assert_eq!(content.parts[0].truncated, truncated);
        assert!(content.next_cursor.is_none());
    }
}
