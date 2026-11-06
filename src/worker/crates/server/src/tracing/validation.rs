use std::collections::HashMap;

use litellm_traces::request::{
    CONVERSATION_PAGE_SIZE_MAX, LOG_FILTER_MAX_LENGTH, LOG_PAGE_SIZE_MAX, LogListRequest,
    TRACE_AGENT_NAME_MAX_LENGTH, TRACE_PAGE_SIZE_MAX, TraceConversationRequest, TraceDetailRequest,
    TraceErrorPageRequest, TraceListRequest, TraceSpanRequest,
};
use serde_json::json;

use super::{TraceConfig, TraceReadError, read_error};
use crate::{
    datasets::validation::{Revision, failure, integer},
    error::{DatasetError, TraceHttpError, ValidationError},
};

fn parameters(query: Option<&str>) -> HashMap<String, String> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .into_owned()
        .collect()
}

fn number(
    params: &HashMap<String, String>,
    key: &str,
    errors: &mut Vec<ValidationError>,
) -> Option<Revision> {
    let value = params.get(key)?;
    match integer(&json!(value), &[json!("query"), json!(key)], false) {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(error);
            None
        }
    }
}

fn cursor(params: &HashMap<String, String>, errors: &mut Vec<ValidationError>) -> Option<String> {
    let value = params.get("cursor")?;
    if value.chars().count() > 512 {
        errors.push(failure(
            "string_too_long",
            &[json!("query"), json!("cursor")],
            "String should have at most 512 characters",
            json!(value),
            Some(json!({"max_length":512})),
        ));
    }
    Some(value.clone())
}

fn finish(errors: Vec<ValidationError>) -> Result<(), TraceHttpError> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(DatasetError::Validation(errors).into())
    }
}

fn exact(value: Option<Revision>, config: TraceConfig) -> Result<Option<i64>, TraceHttpError> {
    value
        .map(|value| {
            value
                .exact()
                .ok_or_else(|| read_error(TraceReadError::TooLarge, config))
        })
        .transpose()
}

pub(super) fn list(
    query: Option<&str>,
    config: TraceConfig,
) -> Result<TraceListRequest, TraceHttpError> {
    let params = parameters(query);
    let mut errors = Vec::new();
    let start = number(&params, "start_ms", &mut errors);
    let end = number(&params, "end_ms", &mut errors);
    let cursor = cursor(&params, &mut errors);
    let agent = params.get("agent").cloned();
    let agent_error = agent
        .as_ref()
        .filter(|value| value.chars().count() > TRACE_AGENT_NAME_MAX_LENGTH)
        .map(|value| {
            failure(
                "string_too_long",
                &[json!("query"), json!("agent")],
                format!("String should have at most {TRACE_AGENT_NAME_MAX_LENGTH} characters"),
                json!(value),
                Some(json!({"max_length":TRACE_AGENT_NAME_MAX_LENGTH})),
            )
        });
    errors.extend(agent_error);
    finish(errors)?;
    Ok(TraceListRequest {
        start_ms: exact(start, config)?,
        end_ms: exact(end, config)?,
        cursor,
        agent,
    })
}

pub(super) fn logs(
    query: Option<&str>,
    config: TraceConfig,
) -> Result<LogListRequest, TraceHttpError> {
    let params = parameters(query);
    let mut errors = Vec::new();
    let start = number(&params, "start_ms", &mut errors);
    let end = number(&params, "end_ms", &mut errors);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let retention_ms = config.retention_days.max(1).saturating_mul(86_400_000);
    let start_ms = exact(start, config)?.unwrap_or(now_ms.saturating_sub(86_400_000));
    let end_ms = exact(end, config)?.unwrap_or(now_ms);
    if start_ms >= end_ms || start_ms < now_ms.saturating_sub(retention_ms) || end_ms > now_ms {
        errors.push(failure(
            "invalid_time_range",
            &[json!("query")],
            "Log time range must be ordered and within configured retention",
            json!({"start_ms":start_ms,"end_ms":end_ms}),
            None,
        ));
    }
    let service = bounded_text(&params, "service", LOG_FILTER_MAX_LENGTH, &mut errors);
    let event = bounded_text(&params, "event", LOG_FILTER_MAX_LENGTH, &mut errors);
    let severity = bounded_text(&params, "severity", 32, &mut errors);
    let trace_id = bounded_text(&params, "trace_id", 64, &mut errors);
    let conversation_id = bounded_text(
        &params,
        "conversation_id",
        LOG_FILTER_MAX_LENGTH,
        &mut errors,
    );
    let cursor = params.get("cursor").cloned().filter(|value| {
        if value.len() > 128 {
            errors.push(failure(
                "string_too_long",
                &[json!("query"), json!("cursor")],
                "String should have at most 128 characters",
                json!(value),
                Some(json!({"max_length":128})),
            ));
            false
        } else {
            true
        }
    });
    let configured_page_size = config.list_limit.min(u32::from(LOG_PAGE_SIZE_MAX)).max(1) as u16;
    let page_size =
        number(&params, "page_size", &mut errors).map_or(Some(configured_page_size), |value| {
            if value.negative() || value.exact().is_none_or(|value| value == 0) {
                errors.push(failure(
                    "greater_than_equal",
                    &[json!("query"), json!("page_size")],
                    "Input should be greater than or equal to 1",
                    json!(params["page_size"]),
                    Some(json!({"ge":1})),
                ));
                None
            } else if value
                .exact()
                .is_none_or(|value| value > i64::from(LOG_PAGE_SIZE_MAX))
            {
                errors.push(failure(
                    "less_than_equal",
                    &[json!("query"), json!("page_size")],
                    format!("Input should be less than or equal to {LOG_PAGE_SIZE_MAX}"),
                    json!(params["page_size"]),
                    Some(json!({"le":LOG_PAGE_SIZE_MAX})),
                ));
                None
            } else {
                value.exact().map(|value| value as u16)
            }
        });
    finish(errors)?;
    Ok(LogListRequest {
        start_ms: Some(start_ms),
        end_ms: Some(end_ms),
        service,
        event,
        severity,
        trace_id,
        conversation_id,
        cursor,
        page_size,
    })
}

fn bounded_text(
    params: &HashMap<String, String>,
    key: &str,
    max_length: usize,
    errors: &mut Vec<ValidationError>,
) -> Option<String> {
    let value = params.get(key)?;
    if value.chars().count() > max_length {
        errors.push(failure(
            "string_too_long",
            &[json!("query"), json!(key)],
            format!("String should have at most {max_length} characters"),
            json!(value),
            Some(json!({"max_length":max_length})),
        ));
    }
    Some(value.clone())
}

pub(super) fn agents(
    query: Option<&str>,
    config: TraceConfig,
) -> Result<(Option<i64>, Option<i64>), TraceHttpError> {
    let params = parameters(query);
    let mut errors = Vec::new();
    let start = number(&params, "start_ms", &mut errors);
    let end = number(&params, "end_ms", &mut errors);
    finish(errors)?;
    Ok((exact(start, config)?, exact(end, config)?))
}

pub(super) fn detail(query: Option<&str>) -> Result<TraceDetailRequest, TraceHttpError> {
    paged(query, TRACE_PAGE_SIZE_MAX)
}

pub(super) fn conversation(
    query: Option<&str>,
) -> Result<TraceConversationRequest, TraceHttpError> {
    let request = paged(query, CONVERSATION_PAGE_SIZE_MAX)?;
    Ok(TraceConversationRequest {
        trace_ref: request.trace_ref,
        cursor: request.cursor,
        page_size: request.page_size,
    })
}

fn paged(query: Option<&str>, max_size: u16) -> Result<TraceDetailRequest, TraceHttpError> {
    let params = parameters(query);
    let mut errors = Vec::new();
    let cursor = cursor(&params, &mut errors);
    let page_size = number(&params, "page_size", &mut errors);
    let page_size = page_size.and_then(|value| {
        let (kind, message, context) = if value.negative() || value.exact() == Some(0) {
            (
                "greater_than_equal",
                "Input should be greater than or equal to 1".to_owned(),
                json!({"ge":1}),
            )
        } else if value
            .exact()
            .is_none_or(|value| value > i64::from(max_size))
        {
            (
                "less_than_equal",
                format!("Input should be less than or equal to {max_size}"),
                json!({"le":max_size}),
            )
        } else {
            return value.exact().map(|value| value as u16);
        };
        errors.push(failure(
            kind,
            &[json!("query"), json!("page_size")],
            &message,
            json!(params["page_size"]),
            Some(context),
        ));
        None
    });
    finish(errors)?;
    Ok(TraceDetailRequest {
        trace_ref: params.get("trace_ref").cloned().unwrap_or_default(),
        cursor,
        page_size,
    })
}

pub(super) fn span(query: Option<&str>) -> TraceSpanRequest {
    TraceSpanRequest {
        trace_ref: parameters(query).remove("trace_ref").unwrap_or_default(),
    }
}

pub(super) fn error_page(query: Option<&str>) -> Result<TraceErrorPageRequest, TraceHttpError> {
    let params = parameters(query);
    let mut errors = Vec::new();
    let cursor = cursor(&params, &mut errors);
    finish(errors)?;
    Ok(TraceErrorPageRequest {
        trace_ref: params.get("trace_ref").cloned().unwrap_or_default(),
        cursor,
    })
}
