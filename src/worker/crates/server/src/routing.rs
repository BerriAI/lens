use std::sync::Arc;

use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{MethodRouter, any},
};

use lens_auth::{Authentication, SessionRepository};
use lens_contract::auth::Role;

use crate::{
    auth,
    error::{InvestigationError, SessionError},
};

pub const PAGE_PREFIX: &str = "/lens";
pub const ADMIN_PREFIX: &str = "/lens/admin";
pub const MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;

pub(crate) trait PublicRoutes<S> {
    fn public_route(self, path: &str, method: MethodRouter<S>) -> Self;
    fn admin_route<R: SessionRepository + 'static>(
        self,
        authentication: &Arc<Authentication<R>>,
        path: &str,
        method: MethodRouter<S>,
    ) -> Self;
}

impl<S: Clone + Send + Sync + 'static> PublicRoutes<S> for Router<S> {
    fn public_route(self, path: &str, method: MethodRouter<S>) -> Self {
        self.route(path, method)
            .route(&format!("{path}/"), any(redirect))
    }

    fn admin_route<R: SessionRepository + 'static>(
        self,
        authentication: &Arc<Authentication<R>>,
        path: &str,
        method: MethodRouter<S>,
    ) -> Self {
        self.public_route(
            &format!("{ADMIN_PREFIX}{path}"),
            method.route_layer(axum::middleware::from_fn_with_state(
                authentication.clone(),
                require_admin::<R>,
            )),
        )
    }
}

async fn require_admin<R: SessionRepository>(
    State(authentication): State<Arc<Authentication<R>>>,
    request: Request,
    next: Next,
) -> Response {
    match auth::identity(&authentication, request.headers(), request.method()).await {
        Ok(identity)
            if matches!(
                identity.user_role,
                Role::ProxyAdmin | Role::ProxyAdminViewer
            ) =>
        {
            next.run(request).await
        }
        Ok(_) => InvestigationError::ForbiddenRead.into_response(),
        Err(error) => SessionError::from(error).into_response(),
    }
}

async fn redirect(request: Request) -> Response {
    let path = request.uri().path().trim_end_matches('/');
    let query = request
        .uri()
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let location = if let Some(host) = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    {
        format!(
            "{}://{host}{path}{query}",
            request.uri().scheme_str().unwrap_or("http")
        )
    } else {
        format!("{path}{query}")
    };
    (
        StatusCode::TEMPORARY_REDIRECT,
        [(header::LOCATION, location)],
    )
        .into_response()
}

pub async fn redirect_trailing_slash(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let normalized = path.trim_end_matches('/');
    let parts: Vec<_> = normalized.split('/').collect();
    let agent_io_route = normalized == "/lens/evals"
        || normalized.starts_with("/lens/evals/")
        || (matches!(request.method().as_str(), "GET" | "HEAD")
            && matches!(
                parts.as_slice(),
                ["", "lens", "datasets", _, "revisions", _, "cases"]
            ));
    let contract: Vec<_> = request
        .headers()
        .get_all("x-lens-contract")
        .iter()
        .collect();
    if (path == "/lens" || path.starts_with("/lens/"))
        && !contract.is_empty()
        && contract.as_slice() != ["1"]
        && !(agent_io_route && contract.as_slice() == ["2"])
    {
        return (
            StatusCode::CONFLICT,
            axum::Json(serde_json::json!({"detail":"contract_version","code":"contract_version"})),
        )
            .into_response();
    }
    let known = matches!(
        parts.as_slice(),
        ["", "lens"]
            | ["", "lens", _]
            | ["", "lens", _, "findings", "import"]
            | ["", "lens", "admin", _]
            | ["", "lens", "admin", _, "runs" | "cancel"]
            | ["", "lens", "admin", _, "runs" | "findings", _]
            | ["", "lens", "admin", _, "runs", _, "reviews"]
            | ["", "lens", "admin", _, "executions", _]
            | ["", "lens", "admin", "activity", "available"]
            | ["", "lens", "admin", "preview", "sample"]
            | ["", "lens", "admin", "traces", "findings" | "signals"]
            | ["", "lens", "logs", _]
            | ["", "lens", "admin", "feedback", "summary"]
            | ["", "lens", "admin", "tracing", "keys"]
            | ["", "lens", "admin", "tracing", "keys", _]
            | ["", "lens", "admin", "datasets", _, "revisions" | "export"]
            | ["", "lens", "admin", "datasets", _, "revisions", _, "cases"]
            | ["", "lens", "datasets", _, "revisions", _, "cases"]
            | ["", "lens", "evals", "runs", _, "finish" | "details"]
            | ["", "lens", "evals", "runs", _, "results", _, _]
            | ["", "v1", "traces"]
            | ["", "v1", "traces", _]
            | ["", "v1", "logs"]
            | ["", "v1", "logs", _]
            | ["", "v1", "traces", "query", "help"]
            | ["", "v1", "traces", _, "spans", _]
            | ["", "v1", "traces", _, "spans", _, "error"]
            | ["", "models"]
            | ["", "v1", "models"]
            | ["", "model_group", "info"]
            | ["", "auth", "session"]
    );
    if path != normalized && known {
        return redirect(request).await;
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, routing::get};
    use rstest::rstest;
    use tower::ServiceExt;

    #[rstest]
    #[case::relative("/example/?q=x%2Fy", None, "/example?q=x%2Fy")]
    #[case::host("/example/", Some("lens.test"), "http://lens.test/example")]
    #[case::scheme(
        "https://lens.test/example/",
        Some("lens.test"),
        "https://lens.test/example"
    )]
    #[tokio::test]
    async fn redirects_survive_subsequent_router_merges(
        #[case] uri: &str,
        #[case] host: Option<&str>,
        #[case] location: &str,
    ) {
        let router = Router::new()
            .public_route("/example", get(|| async { StatusCode::NO_CONTENT }))
            .merge(Router::new().route("/later", get(|| async { StatusCode::OK })));
        let request = Request::builder().uri(uri).method("POST");
        let request = match host {
            Some(host) => request.header(header::HOST, host),
            None => request,
        };
        let response = router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], location);
    }

    #[rstest]
    #[case::canonical("/example", 204)]
    #[case::unknown("/unknown/", 404)]
    #[tokio::test]
    async fn canonical_and_unknown_paths_keep_their_handlers(
        #[case] path: &str,
        #[case] status: u16,
    ) {
        let router =
            Router::new().public_route("/example", get(|| async { StatusCode::NO_CONTENT }));
        let response = router
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }
}

#[cfg(test)]
mod composed_tests {
    use super::*;
    use axum::{body::Body, middleware, routing::get};
    use rstest::rstest;
    use tower::ServiceExt;

    #[rstest]
    #[case::eval("POST", "/lens/evals/runs", 204)]
    #[case::read_cases("GET", "/lens/datasets/one/revisions/1/cases", 204)]
    #[case::admin_read("GET", "/lens/admin/datasets/one", 409)]
    #[case::other_mutation("POST", "/lens/admin/datasets/one", 409)]
    #[case::other_api("GET", "/lens/traces", 409)]
    #[tokio::test]
    async fn agent_io_version_is_scoped_to_eval_routes(
        #[case] method: &str,
        #[case] path: &str,
        #[case] status: u16,
    ) {
        let router = Router::new()
            .route(
                path,
                axum::routing::any(|| async { StatusCode::NO_CONTENT }),
            )
            .layer(middleware::from_fn(redirect_trailing_slash));
        let response = router
            .oneshot(
                Request::builder()
                    .uri(path)
                    .method(method)
                    .header("x-lens-contract", "2")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
    }

    #[rstest]
    #[case::legacy(None, 204)]
    #[case::current(Some("1"), 204)]
    #[case::unsupported(Some("2"), 409)]
    #[case::wire_protocol_is_not_public_contract(Some("7"), 409)]
    #[case::malformed(Some("1, 2"), 409)]
    #[tokio::test]
    async fn public_contract_is_checked_before_mutations(
        #[case] version: Option<&str>,
        #[case] status: u16,
    ) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        let router = Router::new()
            .route(
                "/lens/admin/datasets",
                axum::routing::post(move || async move {
                    counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    StatusCode::NO_CONTENT
                }),
            )
            .layer(middleware::from_fn(redirect_trailing_slash));
        let request = Request::builder()
            .method("POST")
            .uri("/lens/admin/datasets");
        let request = match version {
            Some(version) => request.header("x-lens-contract", version),
            None => request,
        };
        let response = router
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), status);
        if status == 409 {
            let body = axum::body::to_bytes(response.into_body(), usize::MAX)
                .await
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
                serde_json::json!({"detail":"contract_version","code":"contract_version"})
            );
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::Relaxed),
            usize::from(status == 204)
        );
    }

    #[rstest]
    #[case::datasets("/lens/admin/datasets///?x=1", "/lens/admin/datasets?x=1")]
    #[case::activity("/lens/admin/activity/available////", "/lens/admin/activity/available")]
    #[case::feedback("/lens/feedback//", "/lens/feedback")]
    #[case::evidence("/lens/admin/one/executions/two//", "/lens/admin/one/executions/two")]
    #[case::traces("/v1/traces/one/spans/two/error//", "/v1/traces/one/spans/two/error")]
    #[tokio::test]
    async fn final_middleware_preserves_repeated_slashes_after_ui_merge(
        #[case] path: &str,
        #[case] target: &str,
    ) {
        let router = Router::new()
            .public_route("/lens/admin/datasets", get(|| async { StatusCode::OK }))
            .merge(Router::new().fallback(|| async { StatusCode::NOT_FOUND }))
            .layer(middleware::from_fn(redirect_trailing_slash));
        let response = router
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(header::HOST, "lens.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers()[header::LOCATION],
            format!("http://lens.test{target}")
        );
    }
}
