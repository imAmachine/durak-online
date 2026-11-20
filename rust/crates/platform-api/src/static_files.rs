use std::path::{Path, PathBuf};

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

fn reject_ambiguous_path(path: &str) -> bool {
    path.split('/').any(|part| {
        part.is_empty() || part == "." || part == ".." || part.contains('\\') || part.contains('%')
    })
}

fn static_response(bytes: Vec<u8>, mime: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, mime),
            (header::CACHE_CONTROL, "no-store, no-cache, must-revalidate"),
        ],
        bytes,
    )
        .into_response()
}

async fn read_from_root(root: &Path, relative: &str, mime: &'static str) -> Response {
    if reject_ambiguous_path(relative) {
        return error(StatusCode::NOT_FOUND, "Not found");
    }

    let Ok(root) = tokio::fs::canonicalize(root).await else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    let Ok(candidate) = tokio::fs::canonicalize(root.join(relative)).await else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    if !candidate.starts_with(&root) {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    match tokio::fs::metadata(&candidate).await {
        Ok(metadata) if metadata.is_file() => match tokio::fs::read(candidate).await {
            Ok(bytes) => static_response(bytes, mime),
            Err(_) => error(StatusCode::NOT_FOUND, "Not found"),
        },
        _ => error(StatusCode::NOT_FOUND, "Not found"),
    }
}

async fn read_from_directory(root: &Path, relative: &str, mime: &'static str) -> Response {
    if reject_ambiguous_path(relative) {
        return error(StatusCode::NOT_FOUND, "Not found");
    }

    let directory = root.join(relative.split('/').next().unwrap_or_default());
    let candidate = root.join(relative);
    let Ok(root) = tokio::fs::canonicalize(root).await else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    let Ok(directory) = tokio::fs::canonicalize(directory).await else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    if !directory.starts_with(&root) || directory == root {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    let Ok(candidate) = tokio::fs::canonicalize(candidate).await else {
        return error(StatusCode::NOT_FOUND, "Not found");
    };
    if !candidate.starts_with(&directory) {
        return error(StatusCode::NOT_FOUND, "Not found");
    }
    match tokio::fs::metadata(&candidate).await {
        Ok(metadata) if metadata.is_file() => match tokio::fs::read(candidate).await {
            Ok(bytes) => static_response(bytes, mime),
            Err(_) => error(StatusCode::NOT_FOUND, "Not found"),
        },
        _ => error(StatusCode::NOT_FOUND, "Not found"),
    }
}

async fn route_from_root(uri: Uri, root: PathBuf) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "Этот legacy endpoint ещё не реализован в Rust API",
        );
    }

    match path {
        "/" | "/index.html" => read_from_root(&root, "index.html", "text/html").await,
        "/admin" | "/admin.html" => read_from_root(&root, "admin.html", "text/html").await,
        "/app.js" => read_from_root(&root, "app.js", "application/javascript").await,
        "/style.css" => read_from_directory(&root, "css/style.css", "text/css").await,
        path if path.starts_with("/js/") && path.ends_with(".js") => {
            read_from_directory(&root, &path[1..], "application/javascript").await
        }
        path if path.starts_with("/css/") && path.ends_with(".css") => {
            read_from_directory(&root, &path[1..], "text/css").await
        }
        _ => error(StatusCode::NOT_FOUND, "Not found"),
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
        std::fs::write(dir.path().join("admin.html"), "custom admin").unwrap();
        std::fs::write(dir.path().join("app.js"), "legacy app").unwrap();
        std::fs::create_dir(dir.path().join("js")).unwrap();
        std::fs::create_dir(dir.path().join("css")).unwrap();
        std::fs::write(dir.path().join("js/app.js"), "console.log('ok')").unwrap();
        std::fs::write(dir.path().join("css/style.css"), "body {} ").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "secret").unwrap();
        let root = static_root(Some(dir.path().as_os_str().to_owned()));
        let response = route_from_root("/".parse().unwrap(), root.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/html");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store, no-cache, must-revalidate"
        );
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "custom index"
        );
        for (path, mime, body) in [
            ("/app.js", "application/javascript", "legacy app"),
            ("/js/app.js", "application/javascript", "console.log('ok')"),
            ("/style.css", "text/css", "body {} "),
            ("/css/style.css", "text/css", "body {} "),
        ] {
            let response = route_from_root(path.parse().unwrap(), root.clone()).await;
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
            assert_eq!(
                response.headers()[header::CACHE_CONTROL],
                "no-store, no-cache, must-revalidate"
            );
            assert_eq!(to_bytes(response.into_body(), 1024).await.unwrap(), body);
        }
        for path in [
            "/secret.txt",
            "/../secret.txt",
            "/server.js",
            "/AGENTS.md",
            "/.git/config",
            "/tests/engine.mjs",
            "/%2e%2e/server.js",
            "/js/../secret.js",
            "/css/../secret.css",
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

    #[cfg(unix)]
    #[tokio::test]
    async fn root_files_cannot_escape_static_root_through_symlinks() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("index.html"), "outside index").unwrap();
        std::fs::write(outside.path().join("admin.html"), "outside admin").unwrap();
        std::fs::write(outside.path().join("app.js"), "outside app").unwrap();
        for name in ["index.html", "admin.html", "app.js"] {
            symlink(outside.path().join(name), root.path().join(name)).unwrap();
        }

        for path in ["/", "/index.html", "/admin", "/admin.html", "/app.js"] {
            assert_eq!(
                route_from_root(path.parse().unwrap(), root.path().to_path_buf())
                    .await
                    .status(),
                StatusCode::NOT_FOUND,
                "unexpected access through {path}"
            );
        }
    }
}
