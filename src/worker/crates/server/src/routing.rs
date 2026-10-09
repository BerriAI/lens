use axum::{
    Router,
    extract::Request,
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{MethodRouter, any},
};

pub(crate) trait PublicRoutes<S> {
    fn public_route(self, path: &str, method: MethodRouter<S>) -> Self;
}

impl<S: Clone + Send + Sync + 'static> PublicRoutes<S> for Router<S> {
    fn public_route(self, path: &str, method: MethodRouter<S>) -> Self {
        self.route(path, method)
            .route(&format!("{path}/"), any(redirect))
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
    let contract: Vec<_> = request
        .headers()
        .get_all("x-lens-contract")
        .iter()
        .collect();
    if (path == "/lens" || path.starts_with("/lens/"))
        && !contract.is_empty()
        && contract.as_slice() != ["1"]
    {
        return (
            StatusCode::CONFLICT,
            axum::Json(serde_json::json!({"detail":"contract_version","code":"contract_version"})),
        )
            .into_response();
    }
    let normalized = path.trim_end_matches('/');
    let parts: Vec<_> = normalized.split('/').collect();
    let known = matches!(
        parts.as_slice(),
        ["", "lens"]
            | ["", "lens", _]
            | ["", "lens", _, "runs" | "cancel"]
            | ["", "lens", _, "runs" | "findings" | "executions", _]
            | ["", "lens", _, "runs", _, "reviews"]
            | ["", "lens", "activity", "available"]
            | ["", "lens", "preview", "sample"]
            | ["", "lens", "traces", "findings" | "signals"]
            | ["", "lens", "feedback", "summary"]
            | ["", "lens", "tracing", "keys"]
            | ["", "lens", "tracing", "keys", _]
            | ["", "lens", "datasets", _]
            | ["", "lens", "datasets", _, "revisions" | "export"]
            | ["", "lens", "datasets", _, "revisions", _, "cases"]
            | ["", "lens", "evals", "runs", _, "finish" | "details"]
            | ["", "lens", "evals", "runs", _, "results", _, _]
            | ["", "v1", "traces"]
            | ["", "v1", "traces", _]
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
                "/lens/datasets",
                axum::routing::post(move || async move {
                    counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    StatusCode::NO_CONTENT
                }),
            )
            .layer(middleware::from_fn(redirect_trailing_slash));
        let request = Request::builder().method("POST").uri("/lens/datasets");
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
    #[case::datasets("/lens/datasets///?x=1", "/lens/datasets?x=1")]
    #[case::activity("/lens/activity/available////", "/lens/activity/available")]
    #[case::feedback("/lens/feedback//", "/lens/feedback")]
    #[case::evidence("/lens/one/executions/two//", "/lens/one/executions/two")]
    #[case::traces("/v1/traces/one/spans/two/error//", "/v1/traces/one/spans/two/error")]
    #[tokio::test]
    async fn final_middleware_preserves_repeated_slashes_after_ui_merge(
        #[case] path: &str,
        #[case] target: &str,
    ) {
        let router = Router::new()
            .public_route("/lens/datasets", get(|| async { StatusCode::OK }))
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
