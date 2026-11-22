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

#[fixture]
fn built() -> (Router, TempDir) {
    let directory = tempfile::tempdir().unwrap();
    let public = directory.path().join("ui");
    std::fs::create_dir_all(public.join("_next/static")).unwrap();
    std::fs::write(
        public.join("index.html"),
        "<script src=\"/__lens_ui_mount__/_next/static/app.js\"></script>",
    )
    .unwrap();
    std::fs::write(
        public.join("_next/static/app.js"),
        "fetch(\"/__lens_ui_mount__/_next/static/chunk.js\")",
    )
    .unwrap();
    std::fs::write(public.join("_next/static/font.woff2"), [0xff, 0xfe, 0x00]).unwrap();
    std::fs::write(directory.path().join("secret.txt"), "outside-static-root").unwrap();
    (lens_server::ui::router(public), directory)
}

async fn mounted(router: Router, path: &str, mount: Option<&str>) -> axum::response::Response {
    let request = Request::builder().uri(path);
    let request = match mount {
        Some(mount) => request.header(lens_server::ui::MOUNT_HEADER, mount),
        None => request,
    };
    router
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap()
}

#[rstest]
#[case::gateway_root(Some("/lens/ui"), "/lens/ui")]
#[case::gateway_under_a_root_path(Some("/team-a/lens/ui"), "/team-a/lens/ui")]
#[case::opened_on_lens_itself(None, "/ui")]
#[tokio::test]
async fn one_build_loads_its_assets_from_where_it_is_mounted(
    built: (Router, TempDir),
    #[case] mount: Option<&str>,
    #[case] expected: &str,
) {
    let page = mounted(built.0.clone(), "/ui/", mount).await;
    assert_eq!(page.status(), StatusCode::OK);
    assert_eq!(page.headers()[header::CACHE_CONTROL], "no-cache");
    assert_eq!(
        to_bytes(page.into_body(), 1024).await.unwrap(),
        format!("<script src=\"{expected}/_next/static/app.js\"></script>")
    );
    let script = mounted(built.0, "/ui/_next/static/app.js", mount).await;
    assert_eq!(
        to_bytes(script.into_body(), 1024).await.unwrap(),
        format!("fetch(\"{expected}/_next/static/chunk.js\")")
    );
}

#[rstest]
#[case::page("/ui/")]
#[case::versioned_asset("/ui/_next/static/app.js")]
#[tokio::test]
async fn a_cache_keeps_each_mount_apart(built: (Router, TempDir), #[case] path: &str) {
    let response = mounted(built.0, path, Some("/lens/ui")).await;
    assert_eq!(
        response.headers()[header::VARY],
        lens_server::ui::MOUNT_HEADER
    );
    assert!(!response.headers().contains_key(header::ETAG));
}

#[rstest]
#[case::gateway(Some("/lens/ui"), "/lens/ui/?trace=one")]
#[case::opened_on_lens_itself(None, "/ui/?trace=one")]
#[tokio::test]
async fn the_entry_redirect_stays_inside_the_mount(
    built: (Router, TempDir),
    #[case] mount: Option<&str>,
    #[case] location: &str,
) {
    let response = mounted(built.0, "/ui?trace=one", mount).await;
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.headers()[header::LOCATION], location);
}

#[rstest]
#[case::gateway(Some("/lens/ui"), Some("/lens/ui"))]
#[case::opened_on_lens_itself(None, None)]
#[tokio::test]
async fn a_host_can_tell_that_this_lens_mounted_the_ui_for_it(
    built: (Router, TempDir),
    #[case] mount: Option<&str>,
    #[case] echoed: Option<&str>,
) {
    let response = mounted(built.0, "/ui/", mount).await;
    assert_eq!(
        response
            .headers()
            .get(lens_server::ui::MOUNT_HEADER)
            .map(|value| value.to_str().unwrap()),
        echoed
    );
}

#[rstest]
#[tokio::test]
async fn binary_assets_are_served_unchanged(built: (Router, TempDir)) {
    let response = mounted(built.0, "/ui/_next/static/font.woff2", Some("/lens/ui")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
        [0xff, 0xfe, 0x00]
    );
}

#[rstest]
#[case::script("/lens/ui\"><script>alert(1)</script>")]
#[case::another_origin("//evil.example/ui")]
#[case::parent("/lens/../ui")]
#[case::relative("lens/ui")]
#[case::trailing_slash("/lens/ui/")]
#[tokio::test]
async fn refuses_a_mount_that_is_not_a_plain_path(built: (Router, TempDir), #[case] mount: &str) {
    let response = mounted(built.0, "/ui/", Some(mount)).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[rstest]
#[case::parent("/ui/../secret.txt")]
#[case::encoded_parent("/ui/%2e%2e/secret.txt")]
#[case::missing_asset("/ui/missing.js")]
#[tokio::test]
async fn a_mounted_ui_never_serves_files_outside_its_directory(
    built: (Router, TempDir),
    #[case] path: &str,
) {
    let response = mounted(built.0, path, Some("/lens/ui")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
