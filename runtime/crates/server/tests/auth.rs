use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderValue, Request, StatusCode, header::AUTHORIZATION},
    routing::get,
};
use rstest::rstest;
use tower::ServiceExt;

use lens_server::auth::BearerToken;

async fn token_value(token: BearerToken) -> String {
    token.token().to_owned()
}

#[rstest]
#[case::valid(Some("Bearer sample-token"), StatusCode::OK, "sample-token")]
#[case::case_insensitive_scheme(Some("bearer sample-token"), StatusCode::OK, "sample-token")]
#[case::missing_header(
    None,
    StatusCode::UNAUTHORIZED,
    r#"{"detail":"Not authenticated","code":"unauthorized"}"#
)]
#[case::basic_scheme(
    Some("Basic sample-token"),
    StatusCode::UNAUTHORIZED,
    r#"{"detail":"Not authenticated","code":"unauthorized"}"#
)]
#[case::empty_token(
    Some("Bearer "),
    StatusCode::UNAUTHORIZED,
    r#"{"detail":"Not authenticated","code":"unauthorized"}"#
)]
#[case::missing_space(
    Some("Bearer"),
    StatusCode::UNAUTHORIZED,
    r#"{"detail":"Not authenticated","code":"unauthorized"}"#
)]
#[tokio::test]
async fn bearer_token_extractor_accepts_only_nonempty_bearer_credentials(
    #[case] authorization: Option<&str>,
    #[case] expected_status: StatusCode,
    #[case] expected_body: &str,
) {
    let app = Router::new().route("/", get(token_value));
    let mut request = Request::builder().uri("/").body(Body::empty()).unwrap();
    if let Some(value) = authorization {
        request
            .headers_mut()
            .insert(AUTHORIZATION, HeaderValue::from_str(value).unwrap());
    }
    let response = app.oneshot(request).await.unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();

    assert_eq!(status, expected_status);
    assert_eq!(body.as_ref(), expected_body.as_bytes());
}
