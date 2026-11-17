use axum::{
    body::Body,
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
    Router,
};
use std::path::PathBuf;

pub fn router() -> Router {
    Router::new().fallback(any(route))
}

async fn route(uri: Uri) -> Response {
    let path = uri.path();
    if path == "/api/status" {
        return (
            [(header::CONTENT_TYPE, "application/json")],
            r#"{"online":0,"clients":[],"rooms":[]}"#,
        )
            .into_response();
    }
    let (file, mime) = match path {
        "/" | "/index.html" => ("index.html", "text/html"),
        "/app.js" => ("app.js", "application/javascript"),
        "/style.css" => ("style.css", "text/css"),
        _ => return not_found(),
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    match tokio::fs::read(root.join(file)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime)], Body::from(bytes)).into_response(),
        Err(_) => not_found(),
    }
}

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        r#"{"error":"Not found"}"#,
    )
        .into_response()
}
