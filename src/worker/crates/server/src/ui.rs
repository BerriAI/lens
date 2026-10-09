use std::path::PathBuf;

use axum::{
    Router,
    http::{HeaderValue, Uri, header},
    response::Redirect,
    routing::get,
};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

pub fn router(directory: PathBuf) -> Router {
    Router::new()
        .route("/", get(entry))
        .route("/ui", get(entry))
        .nest_service("/ui/", ServeDir::new(directory))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
}

async fn entry(uri: Uri) -> Redirect {
    let location = match uri.query() {
        Some(query) => format!("/ui/?{query}"),
        None => "/ui/".into(),
    };
    Redirect::temporary(&location)
}
