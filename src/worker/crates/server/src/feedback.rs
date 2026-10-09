use crate::routing::PublicRoutes;
pub(crate) mod validation;

pub use crate::error::{FeedbackError, FeedbackStoreError};
use crate::{auth, datasets::validation as body_validation};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, RawQuery, State},
    http::{HeaderMap, Method, StatusCode},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use lens_auth::{Authentication, SessionRepository};
use lens_contract::{
    auth::{Identity, Role},
    feedback::{
        Feedback, FeedbackDeletion, FeedbackSubmission, FeedbackTarget, TraceFeedback,
        TraceFeedbackRequest, TraceFeedbackSummary, TraceIdentity,
    },
    investigations::Scope,
};
use sha2::{Digest, Sha256};
use std::{future::Future, sync::Arc};

pub struct FeedbackWrite {
    pub trace: TraceIdentity,
    pub author: String,
    pub score: u8,
    pub comment: String,
    pub at: DateTime<Utc>,
}

pub trait FeedbackStore: Send + Sync {
    fn for_trace(
        &self,
        scope: &Scope,
        trace: &TraceIdentity,
    ) -> impl Future<Output = Result<Option<TraceFeedback>, FeedbackStoreError>> + Send;
    fn upsert(
        &self,
        scope: &Scope,
        write: &FeedbackWrite,
    ) -> impl Future<Output = Result<Option<Feedback>, FeedbackStoreError>> + Send;
    fn delete(
        &self,
        scope: &Scope,
        trace: &TraceIdentity,
        author: &str,
        at: DateTime<Utc>,
    ) -> impl Future<Output = Result<Option<bool>, FeedbackStoreError>> + Send;
    fn summaries(
        &self,
        scope: &Scope,
        traces: &[TraceIdentity],
    ) -> impl Future<Output = Result<Vec<TraceFeedbackSummary>, FeedbackStoreError>> + Send;
}

struct App<R, S> {
    authentication: Arc<Authentication<R>>,
    store: S,
}

pub fn router<R: SessionRepository + 'static, S: FeedbackStore + 'static>(
    authentication: Arc<Authentication<R>>,
    store: S,
) -> Router {
    Router::new()
        .public_route(
            "/lens/feedback",
            get(read::<R, S>).put(submit::<R, S>).delete(delete::<R, S>),
        )
        .public_route("/lens/feedback/summary", post(summary::<R, S>))
        .layer(DefaultBodyLimit::disable())
        .layer(axum::middleware::from_fn(
            crate::routing::redirect_trailing_slash,
        ))
        .with_state(Arc::new(App {
            authentication,
            store,
        }))
}

fn read_scope(identity: &Identity) -> Result<Scope, FeedbackError> {
    if !matches!(
        identity.user_role,
        Role::ProxyAdmin | Role::ProxyAdminViewer
    ) {
        return Err(FeedbackError::ForbiddenRead);
    }
    Ok(Scope {
        all_teams: true,
        ..Scope::default()
    })
}

fn write_scope(identity: &Identity) -> Result<Scope, FeedbackError> {
    if identity.user_role == Role::ProxyAdmin {
        return Ok(Scope {
            all_teams: true,
            ..Scope::default()
        });
    }
    if identity.user_role == Role::ProxyAdminViewer {
        return Err(FeedbackError::ForbiddenViewer);
    }
    let team = identity.team_id.as_deref().unwrap_or_default();
    let token = identity.token.as_deref().unwrap_or_default();
    if team.is_empty() && token.is_empty() {
        return Err(FeedbackError::ScopeRequired);
    }
    Ok(Scope {
        team_id: team.into(),
        api_key_hash: if team.is_empty() {
            token.into()
        } else {
            String::new()
        },
        all_teams: false,
    })
}

fn author(identity: &Identity, user: &str) -> Result<String, FeedbackError> {
    Some(user)
        .filter(|user| !user.is_empty())
        .or(identity.user_id.as_deref().filter(|user| !user.is_empty()))
        .or(identity.token.as_deref().filter(|token| !token.is_empty()))
        .map(str::to_owned)
        .ok_or(FeedbackError::AuthorRequired)
}

fn trace(trace_id: Option<String>, session_id: Option<String>, trace_ref: String) -> TraceIdentity {
    let trace_id = trace_id.unwrap_or_else(|| {
        let mut digest = Sha256::new();
        digest.update(b"litellm.claude.session.v1\0");
        digest.update(session_id.unwrap_or_default());
        format!("{:x}", digest.finalize())[..32].to_owned()
    });
    TraceIdentity {
        trace_id,
        trace_ref,
    }
}

fn stored_now() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap()
}

async fn read<R: SessionRepository, S: FeedbackStore>(
    State(app): State<Arc<App<R, S>>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    method: Method,
) -> Result<Json<TraceFeedback>, FeedbackError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let target: FeedbackTarget = validation::query(query.as_deref(), validation::Model::Target)?;
    let target = trace(target.trace_id, target.session_id, target.trace_ref);
    Ok(Json(
        app.store
            .for_trace(&read_scope(&identity)?, &target)
            .await?
            .ok_or(FeedbackError::TraceNotFound)?,
    ))
}

async fn submit<R: SessionRepository, S: FeedbackStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Feedback>, FeedbackError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: FeedbackSubmission = body_validation::request(
        parsed,
        body_validation::Model::Feedback(validation::Model::Submission),
    )?;
    let scope = write_scope(&identity)?;
    let write = FeedbackWrite {
        trace: trace(request.trace_id, request.session_id, request.trace_ref),
        author: author(&identity, &request.user)?,
        score: request.score,
        comment: request.comment,
        at: stored_now(),
    };
    Ok(Json(
        app.store
            .upsert(&scope, &write)
            .await?
            .ok_or(FeedbackError::TraceNotFound)?,
    ))
}

async fn delete<R: SessionRepository, S: FeedbackStore>(
    State(app): State<Arc<App<R, S>>>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    method: Method,
) -> Result<StatusCode, FeedbackError> {
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let target: FeedbackDeletion =
        validation::query(query.as_deref(), validation::Model::Deletion)?;
    let scope = write_scope(&identity)?;
    let author = author(&identity, &target.user)?;
    let target = trace(target.trace_id, target.session_id, target.trace_ref);
    match app
        .store
        .delete(&scope, &target, &author, stored_now())
        .await?
    {
        None => Err(FeedbackError::TraceNotFound),
        Some(false) => Err(FeedbackError::FeedbackNotFound),
        Some(true) => Ok(StatusCode::NO_CONTENT),
    }
}

async fn summary<R: SessionRepository, S: FeedbackStore>(
    State(app): State<Arc<App<R, S>>>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Json<Vec<TraceFeedbackSummary>>, FeedbackError> {
    let parsed = body_validation::parse_body(&body, &headers)?;
    let identity = auth::identity(&app.authentication, &headers, &method).await?;
    let request: TraceFeedbackRequest = body_validation::request(
        parsed,
        body_validation::Model::Feedback(validation::Model::Summary),
    )?;
    Ok(Json(
        app.store
            .summaries(&read_scope(&identity)?, &request.traces)
            .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    #[rstest]
    #[case::explicit("reviewer", Some("caller"), Some("key"), Some("reviewer"))]
    #[case::user("", Some("caller"), Some("key"), Some("caller"))]
    #[case::key("", None, Some("key"), Some("key"))]
    #[case::empty_user("", Some(""), Some("key"), Some("key"))]
    #[case::missing("", None, None, None)]
    #[case::whitespace(" ", Some("caller"), None, Some(" "))]
    fn authors_follow_the_existing_fallback_order(
        #[case] user: &str,
        #[case] id: Option<&str>,
        #[case] token: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let identity = Identity {
            user_id: id.map(str::to_owned),
            token: token.map(str::to_owned),
            ..Identity::default()
        };
        let actual = author(&identity, user);
        assert_eq!(actual.as_ref().ok().map(String::as_str), expected);
        if expected.is_none() {
            assert!(matches!(actual, Err(FeedbackError::AuthorRequired)));
        }
    }

    #[rstest]
    #[case::admin(Role::ProxyAdmin,None,None,Some(Scope{all_teams:true,team_id:String::new(),api_key_hash:String::new()}))]
    #[case::viewer(Role::ProxyAdminViewer, Some("alpha"), Some("key"), None)]
    #[case::team(Role::InternalUser,Some("alpha"),Some("key"),Some(Scope{all_teams:false,team_id:"alpha".into(),api_key_hash:String::new()}))]
    #[case::key(Role::InternalUser,None,Some("key"),Some(Scope{all_teams:false,team_id:String::new(),api_key_hash:"key".into()}))]
    #[case::missing(Role::InternalUser, None, None, None)]
    fn write_scope_comes_only_from_the_authenticated_identity(
        #[case] role: Role,
        #[case] team: Option<&str>,
        #[case] key: Option<&str>,
        #[case] expected: Option<Scope>,
    ) {
        let identity = Identity {
            user_role: role,
            team_id: team.map(str::to_owned),
            token: key.map(str::to_owned),
            ..Identity::default()
        };
        assert_eq!(write_scope(&identity).ok(), expected);
    }

    #[rstest]
    fn stored_feedback_time_matches_clickhouse_millisecond_precision() {
        let at = stored_now();
        assert_eq!(at.timestamp_subsec_nanos() % 1_000_000, 0);
        assert!((Utc::now() - at) < chrono::TimeDelta::seconds(1));
    }
}
