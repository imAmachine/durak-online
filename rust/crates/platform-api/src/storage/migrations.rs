use rusqlite::{params, Connection, Result as SqlResult};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMigration {
    pub version: i64,
    pub name: String,
}

/// Applies numbered SQL files exactly once, in lexical order.
pub fn run_migrations(
    connection: &mut Connection,
    directory: impl AsRef<Path>,
) -> SqlResult<Vec<AppliedMigration>> {
    connection.execute_batch("CREATE TABLE IF NOT EXISTS _platform_schema_migrations (version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL);")?;
    let mut files: Vec<PathBuf> = fs::read_dir(directory)
        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sql"))
        .collect();
    files.sort();
    let transaction = connection.transaction()?;
    let mut applied = Vec::new();
    for path in files {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_owned();
        let version = name
            .split('_')
            .next()
            .and_then(|v| v.parse::<i64>().ok())
            .ok_or_else(|| {
                rusqlite::Error::InvalidParameterName(format!("invalid migration filename: {name}"))
            })?;
        if transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM _platform_schema_migrations WHERE version = ?1)",
            [version],
            |r| r.get::<_, bool>(0),
        )? {
            continue;
        }
        let sql = fs::read_to_string(&path)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        transaction.execute_batch(&sql)?;
        transaction.execute("INSERT INTO _platform_schema_migrations(version, name, applied_at) VALUES (?1, ?2, unixepoch())", params![version, name])?;
        applied.push(AppliedMigration { version, name });
    }
    transaction.commit()?;
    Ok(applied)
}

/// Opens SQLite with the durability/concurrency settings required by the repository.
pub fn open_and_migrate(
    path: impl AsRef<Path>,
    migrations: impl AsRef<Path>,
) -> SqlResult<Connection> {
    let connection = Connection::open(path)?;
    configure(&connection)?;
    let mut connection = connection;
    run_migrations(&mut connection, migrations)?;
    Ok(connection)
}

pub fn configure(connection: &Connection) -> SqlResult<()> {
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;",
    )
}
