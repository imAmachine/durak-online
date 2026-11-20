use std::{env, error::Error, fs, path::PathBuf};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let db_path =
        PathBuf::from(env::var("DURAK_DB").unwrap_or_else(|_| "data/durak.sqlite".into()));
    if let Some(parent) = db_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let migrations = migrations_dir(env::var_os("DURAK_MIGRATIONS_DIR"));
    let repository = platform_api::storage::Repository::open(&db_path, migrations)?;
    eprintln!("SQLite ready at {}", db_path.display());

    let state = platform_api::AppState::with_repository(repository)?.with_discord_from_env()?;
    let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into());
    let port = env::var("PORT").unwrap_or_else(|_| "4173".into());
    let listener = tokio::net::TcpListener::bind(format!("{host}:{port}")).await?;
    eprintln!("Listening on {}", listener.local_addr()?);
    axum::serve(
        listener,
        platform_api::router_with_state(state)
            .into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

fn migrations_dir(value: Option<std::ffi::OsString>) -> PathBuf {
    value
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_directory_override_and_default() {
        assert_eq!(
            migrations_dir(Some("custom/migrations".into())),
            PathBuf::from("custom/migrations")
        );
        assert_eq!(
            migrations_dir(None),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
        );
    }
}
