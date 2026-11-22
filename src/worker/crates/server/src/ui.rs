use std::path::PathBuf;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderValue, StatusCode, Uri, header},
    middleware::{Next, from_fn},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use tower_http::{services::ServeDir, set_header::SetResponseHeaderLayer};

pub const MOUNT_HEADER: &str = "x-lens-ui-mount";
pub const MOUNT_PLACEHOLDER: &str = "/__lens_ui_mount__";
const OWN_PATH: &str = "/ui";
const MAX_ASSET_BYTES: usize = 32 * 1024 * 1024;

pub fn router(directory: PathBuf) -> Router {
    Router::new()
        .route("/", get(entry))
        .route("/ui", get(entry))
        .nest_service("/ui/", ServeDir::new(directory))
        .layer(from_fn(mount))
        .layer(nosniff())
}

fn nosniff() -> SetResponseHeaderLayer<HeaderValue> {
    SetResponseHeaderLayer::if_not_present(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    )
}

async fn entry(uri: Uri) -> Redirect {
    let location = match uri.query() {
        Some(query) => format!("/ui/?{query}"),
        None => "/ui/".into(),
    };
    Redirect::temporary(&location)
}

fn mount_path(value: Option<&HeaderValue>) -> Option<String> {
    let Some(value) = value else {
        return Some(OWN_PATH.into());
    };
    let path = value.to_str().ok()?;
    let segments = path.strip_prefix('/')?.split('/');
    let valid = path.len() <= 256
        && segments.into_iter().all(|segment| {
            !segment.is_empty()
                && segment.bytes().any(|byte| byte != b'.')
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        });
    valid.then(|| path.into())
}

async fn mount(mut request: Request, next: Next) -> Response {
    if !request.uri().path().starts_with("/ui/") {
        return next.run(request).await;
    }
    let Some(path) = mount_path(request.headers().get(MOUNT_HEADER)) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let hosted = request.headers().get(MOUNT_HEADER).cloned();
    let versioned = request.uri().path().starts_with("/ui/_next/static/");
    for name in [
        header::RANGE,
        header::IF_RANGE,
        header::IF_MODIFIED_SINCE,
        header::IF_NONE_MATCH,
    ] {
        request.headers_mut().remove(name);
    }
    let (mut parts, body) = next.run(request).await.into_parts();
    let Ok(bytes) = to_bytes(body, MAX_ASSET_BYTES).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    if let Some(hosted) = hosted {
        parts.headers.insert(MOUNT_HEADER, hosted);
    }
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    let mounted = text.replace(MOUNT_PLACEHOLDER, &path);
    for name in [
        header::CONTENT_LENGTH,
        header::LAST_MODIFIED,
        header::ACCEPT_RANGES,
    ] {
        parts.headers.remove(name);
    }
    parts.headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static(if versioned && parts.status.is_success() {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        }),
    );
    Response::from_parts(parts, Body::from(mounted))
}
