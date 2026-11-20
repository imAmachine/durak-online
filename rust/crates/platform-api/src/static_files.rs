use std::path::PathBuf;

use axum::{
    http::{header, StatusCode, Uri},
    response::{IntoResponse, Response},
};

use crate::error;

fn static_root(value: Option<std::ffi::OsString>) -> PathBuf {
    value
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

pub(crate) async fn route(uri: Uri) -> Response {
    let root = static_root(std::env::var_os("DURAK_STATIC_DIR"));
    route_from_root(uri, root).await
}

async fn route_from_root(uri: Uri, root: PathBuf) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "Этот legacy endpoint ещё не реализован в Rust API",
        );
    }
    let (file, mime) = match path {
        "/" | "/index.html" => ("index.html", "text/html"),
        "/admin" => ("admin.html", "text/html"),
        "/admin.html" => ("admin.html", "text/html"),
        "/app.js" => ("app.js", "application/javascript"),
        "/style.css" => ("style.css", "text/css"),
        _ => return error(StatusCode::NOT_FOUND, "Not found"),
    };
    match tokio::fs::read(root.join(file)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        Err(_) => error(StatusCode::NOT_FOUND, "Not found"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn static_paths_keep_allowlist() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "custom index").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
        let root = static_root(Some(dir.path().as_os_str().to_owned()));
        let response = route_from_root("/".parse().unwrap(), root.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "custom index"
        );
        for path in [
            "/secret.txt",
            "/../secret.txt",
            "/admin",
            "/server.js",
            "/AGENTS.md",
            "/.git/config",
            "/tests/engine.mjs",
            "/%2e%2e/server.js",
        ] {
            assert_eq!(
                route_from_root(path.parse().unwrap(), root.clone())
                    .await
                    .status(),
                StatusCode::NOT_FOUND
            );
        }
        assert_eq!(
            static_root(None),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
        );
    }
}
