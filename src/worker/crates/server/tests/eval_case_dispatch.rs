use axum::{Router, body::Body, http::Request, routing::get};
use lens_server::evals::with_contract_cases;
use rstest::rstest;
use tower::ServiceExt;

const CASES: &str = "/lens/datasets/{id}/revisions/{revision}/cases";

fn app() -> Router {
    let admin = Router::new()
        .route(CASES, get(|| async { "admin" }))
        .route("/lens/datasets/{id}", get(|| async { "admin" }));
    let contract = Router::new().route(CASES, get(|| async { "contract" }));
    with_contract_cases(admin, contract)
}

#[rstest]
#[case::contract_case_read("/lens/datasets/d1/revisions/7/cases", true, "contract")]
#[case::admin_case_read("/lens/datasets/d1/revisions/7/cases", false, "admin")]
#[case::contract_other_dataset_route("/lens/datasets/d1", true, "admin")]
#[tokio::test]
async fn should_route_only_contract_case_reads_to_the_eval_api(
    #[case] path: &str,
    #[case] contract: bool,
    #[case] expected: &str,
) {
    let request = Request::get(path);
    let request = if contract {
        request.header("X-Lens-Contract", "1")
    } else {
        request
    };
    let response = app()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(body, expected);
}
