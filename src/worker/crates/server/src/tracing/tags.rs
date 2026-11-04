use axum::{
    Json,
    body::Bytes,
    extract::{Path, RawQuery, State},
    http::{HeaderMap, Method},
};
use lens_auth::SessionRepository;
use lens_contract::auth::Role;
use litellm_traces::request::{TraceTags, TraceTagsUpdate};
use std::sync::Arc;

use super::{App, TraceReader, auth, read_error, scope, validation};
use crate::error::TraceHttpError;

fn editable(role: Role) -> bool {
    matches!(
        role,
        Role::ProxyAdmin | Role::InternalUser | Role::Team | Role::OrgAdmin
    )
}

pub(super) async fn read<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
    Path(trace_id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Json<TraceTags>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request = validation::span(query.as_deref());
    let mut tags = app
        .reader
        .tags(scope(&identity)?, trace_id.clone(), request.trace_ref, None)
        .await
        .map_err(|error| read_error(error, app.config))?
        .ok_or(TraceHttpError::MissingTrace(trace_id))?;
    tags.editable = editable(identity.user_role);
    Ok(Json(tags))
}

pub(super) async fn write<R: SessionRepository, B: TraceReader>(
    State(app): State<Arc<App<R, B>>>,
    headers: HeaderMap,
    method: Method,
    Path(trace_id): Path<String>,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Result<Json<TraceTags>, TraceHttpError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    if !editable(identity.user_role) {
        return Err(TraceHttpError::Forbidden(
            "This role cannot edit trace tags",
        ));
    }
    let update = serde_json::from_slice::<TraceTagsUpdate>(&body).map_err(|_| {
        read_error(
            super::TraceReadError::InvalidRequest("Expected a tags array".into()),
            app.config,
        )
    })?;
    let update = TraceTagsUpdate {
        tags: update
            .tags
            .into_iter()
            .map(|tag| litellm_traces::request::TraceTag {
                key: tag.key.trim().into(),
                value: tag.value.trim().into(),
            })
            .collect(),
    };
    if !validation::valid_tags(&update.tags)
        || update.tags.iter().enumerate().any(|(index, tag)| {
            update.tags[..index]
                .iter()
                .any(|other| other.key == tag.key)
        })
    {
        return Err(read_error(
            super::TraceReadError::InvalidRequest(
                "Use at most 8 tags with unique, nonempty keys and values up to 256 characters"
                    .into(),
            ),
            app.config,
        ));
    }
    let request = validation::span(query.as_deref());
    let mut tags = app
        .reader
        .tags(
            scope(&identity)?,
            trace_id.clone(),
            request.trace_ref,
            Some(update.tags),
        )
        .await
        .map_err(|error| read_error(error, app.config))?
        .ok_or(TraceHttpError::MissingTrace(trace_id))?;
    tags.editable = true;
    Ok(Json(tags))
}
