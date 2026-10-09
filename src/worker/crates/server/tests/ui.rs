use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use rstest::{fixture, rstest};
use tempfile::TempDir;
use tower::ServiceExt;

#[fixture]
fn ui() -> (Router, TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let public = directory.path().join("ui");
    std::fs::create_dir_all(public.join("_next/static")).unwrap();
    std::fs::write(public.join("index.html"), "<title>Lens</title>").unwrap();
    std::fs::write(public.join("_next/static/app.js"), "window.lens = true;").unwrap();
    std::fs::write(directory.path().join("secret.txt"), "outside-static-root").unwrap();
    (lens_server::ui::router(public), directory)
}

#[rstest]
#[case::root("/", "/ui/")]
#[case::ui_without_slash("/ui", "/ui/")]
#[case::deep_link("/ui?trace=one&trace_ref=scope", "/ui/?trace=one&trace_ref=scope")]
#[tokio::test]
async fn entry_paths_redirect_to_the_static_ui(
    ui: (Router, TempDir),
    #[case] path: &str,
    #[case] location: &str,
) {
    let response =
        ui.0.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
    assert!(response.status().is_redirection());
    assert_eq!(response.headers()[header::LOCATION], location);
}

#[rstest]
#[case::index("/ui/", "<title>Lens</title>")]
#[case::deep_link("/ui/?trace=one&trace_ref=scope", "<title>Lens</title>")]
#[case::asset("/ui/_next/static/app.js", "window.lens = true;")]
#[tokio::test]
async fn serves_bundled_ui_and_assets(
    ui: (Router, TempDir),
    #[case] path: &str,
    #[case] expected: &str,
) {
    let response =
        ui.0.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        expected
    );
}

#[rstest]
#[case::parent("/ui/../secret.txt")]
#[case::encoded_parent("/ui/%2e%2e/secret.txt")]
#[case::nested_parent("/ui/_next/../../secret.txt")]
#[case::missing_asset("/ui/missing.js")]
#[case::api("/auth/unknown")]
#[tokio::test]
async fn never_serves_files_outside_ui_or_html_for_missing_apis(
    ui: (Router, TempDir),
    #[case] path: &str,
) {
    let response =
        ui.0.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), 1024).await.unwrap();
    assert_ne!(body, "outside-static-root");
    assert_ne!(body, "<title>Lens</title>");
}

#[rstest]
#[tokio::test]
async fn javascript_asset_has_an_executable_mime_type(ui: (Router, TempDir)) {
    let response =
        ui.0.oneshot(
            Request::builder()
                .uri("/ui/_next/static/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let content_type = response.headers()[header::CONTENT_TYPE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap();
    assert!(matches!(
        content_type,
        "text/javascript" | "application/javascript"
    ));
}
