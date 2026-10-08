use std::path::Path;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use rstest::rstest;
use tower::ServiceExt;

use lens_server::{
    python_routes::{parse_line, routes},
    router,
};

const ROUTES: &str = include_str!("../python_routes.txt");

#[rstest]
#[tokio::test]
async fn every_python_route_remains_unserved_by_rust() {
    let app = router();

    for line in ROUTES.lines() {
        let (method, path, _) = parse_line(line).expect("Python route entries must be valid");
        let path = path
            .split('/')
            .map(|segment| {
                if segment.starts_with('{') && segment.ends_with('}') {
                    "sample"
                } else {
                    segment
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert!(
            [StatusCode::NOT_FOUND, StatusCode::METHOD_NOT_ALLOWED].contains(&response.status()),
            "{line}"
        );
    }
}

#[rstest]
#[case::unknown_method("FETCH /lens x.py")]
#[case::missing_leading_slash("GET lens src/litellm_lens/endpoints.py")]
#[case::missing_source("GET /lens")]
#[case::empty_line("")]
#[case::wrong_source_prefix("GET /lens src/other/x.py")]
#[case::source_path_traversal("GET /lens src/litellm_lens/../auth.py")]
#[case::path_with_whitespace("GET /lens\tbad src/litellm_lens/endpoints.py")]
#[case::wrong_file_extension("GET /lens src/litellm_lens/endpoints.txt")]
fn malformed_python_route_lines_are_rejected(#[case] line: &str) {
    assert!(parse_line(line).is_none());
}

#[rstest]
#[case::connect("CONNECT")]
#[case::delete("DELETE")]
#[case::get("GET")]
#[case::head("HEAD")]
#[case::options("OPTIONS")]
#[case::patch("PATCH")]
#[case::post("POST")]
#[case::put("PUT")]
#[case::trace("TRACE")]
fn supported_http_methods_are_parsed(#[case] method: &str) {
    let line = format!("{method} /lens src/litellm_lens/endpoints.py");
    let parsed = parse_line(&line).expect("supported HTTP methods must parse");

    assert_eq!(parsed.0.as_str(), method);
}

#[rstest]
fn every_python_route_line_parses_and_points_to_a_source_file() {
    let repository_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");

    assert_eq!(routes().count(), ROUTES.lines().count());
    for (method, path, file) in routes() {
        assert!(
            repository_root.join(file).is_file(),
            "{method} {path} references missing source {file}"
        );
    }
}
