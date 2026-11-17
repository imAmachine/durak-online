use rusqlite::{params, Connection, Result as SqlResult, Transaction, TransactionBehavior};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchState {
    pub id: String,
    pub table_id: String,
    pub game_type: String,
    pub rules_version: String,
    pub mode: String,
    pub status: String,
    pub state_json: String,
    pub seq: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub finished_at: Option<i64>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameEvent {
    pub id: String,
    pub match_id: String,
    pub seq: i64,
    pub public_json: String,
    pub created_at: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerEntry {
    pub id: String,
    pub user_id: String,
    pub game_id: Option<String>,
    pub kind: String,
    pub amount: i64,
    pub operation_id: String,
    pub created_at: i64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchWrite {
    pub state: MatchState,
    pub event: Option<GameEvent>,
    pub ledger: Vec<LedgerEntry>,
}

#[derive(Debug)]
pub enum StorageError {
    Sqlite(rusqlite::Error),
    IdempotencyConflict { kind: &'static str, key: String },
}
impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(e) => e.fmt(f),
            Self::IdempotencyConflict { kind, key } => {
                write!(f, "{kind} idempotency key has a different payload: {key}")
            }
        }
    }
}
impl std::error::Error for StorageError {}
impl From<rusqlite::Error> for StorageError {
    fn from(e: rusqlite::Error) -> Self {
        Self::Sqlite(e)
    }
}
pub type StorageResult<T> = std::result::Result<T, StorageError>;

pub struct Repository {
    connection: Connection,
}
impl Repository {
    pub fn new(connection: Connection) -> StorageResult<Self> {
        configure(&connection)?;
        Ok(Self { connection })
    }
    pub fn open(path: impl AsRef<Path>, migrations: impl AsRef<Path>) -> StorageResult<Self> {
        Ok(Self {
            connection: open_and_migrate(path, migrations)?,
        })
    }
    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    /// Atomically persists the match snapshot, its event, and all wallet entries.
    /// Repeating an identical event or operation is a no-op; reusing its key with
    /// different data returns `IdempotencyConflict` and rolls back the whole batch.
    pub fn save_match(&mut self, write: &MatchWrite) -> StorageResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        upsert_match(&tx, &write.state)?;
        if let Some(event) = &write.event {
            insert_event(&tx, event)?;
        }
        for entry in &write.ledger {
            insert_ledger(&tx, entry)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn save_match_state(&mut self, state: &MatchState) -> StorageResult<()> {
        self.save_match(&MatchWrite {
            state: state.clone(),
            event: None,
            ledger: Vec::new(),
        })
    }
}

fn upsert_match(tx: &Transaction<'_>, m: &MatchState) -> StorageResult<()> {
    let existing: Option<(i64, String, String)> = tx
        .query_row(
            "SELECT seq, state_json, status FROM matches WHERE id=?1",
            [&m.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((seq, state, status)) = existing {
        if seq == m.seq && (state != m.state_json || status != m.status) {
            return Err(StorageError::IdempotencyConflict {
                kind: "match",
                key: m.id.clone(),
            });
        }
        if m.seq < seq {
            return Err(StorageError::IdempotencyConflict {
                kind: "match",
                key: m.id.clone(),
            });
        }
        tx.execute("UPDATE matches SET table_id=?2, game_type=?3, rules_version=?4, mode=?5, status=?6, state_json=?7, seq=?8, updated_at=?9, finished_at=?10 WHERE id=?1", params![m.id,m.table_id,m.game_type,m.rules_version,m.mode,m.status,m.state_json,m.seq,m.updated_at,m.finished_at])?;
    } else {
        tx.execute("INSERT INTO matches(id,table_id,game_type,rules_version,mode,status,state_json,seq,created_at,updated_at,finished_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)", params![m.id,m.table_id,m.game_type,m.rules_version,m.mode,m.status,m.state_json,m.seq,m.created_at,m.updated_at,m.finished_at])?;
    }
    Ok(())
}

fn insert_event(tx: &Transaction<'_>, e: &GameEvent) -> StorageResult<()> {
    let existing: Option<(String, String, i64)> = tx
        .query_row(
            "SELECT id, public_json, created_at FROM game_events WHERE match_id=?1 AND seq=?2",
            params![e.match_id, e.seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((id, json, created_at)) = existing {
        if id != e.id || json != e.public_json || created_at != e.created_at {
            return Err(StorageError::IdempotencyConflict {
                kind: "event",
                key: format!("{}:{}", e.match_id, e.seq),
            });
        }
        return Ok(());
    }
    tx.execute(
        "INSERT INTO game_events(id,match_id,seq,public_json,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![e.id, e.match_id, e.seq, e.public_json, e.created_at],
    )?;
    Ok(())
}

fn insert_ledger(tx: &Transaction<'_>, e: &LedgerEntry) -> StorageResult<()> {
    let existing: Option<(String,String,Option<String>,String,i64,i64)> = tx.query_row("SELECT id,user_id,game_id,kind,amount,created_at FROM wallet_ledger WHERE operation_id=?1", [&e.operation_id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?;
    if let Some((id, user, game, kind, amount, created)) = existing {
        if (id, user, game, kind, amount, created)
            != (
                e.id.clone(),
                e.user_id.clone(),
                e.game_id.clone(),
                e.kind.clone(),
                e.amount,
                e.created_at,
            )
        {
            return Err(StorageError::IdempotencyConflict {
                kind: "ledger",
                key: e.operation_id.clone(),
            });
        }
        return Ok(());
    }
    tx.execute("INSERT INTO wallet_ledger(id,user_id,game_id,kind,amount,operation_id,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![e.id,e.user_id,e.game_id,e.kind,e.amount,e.operation_id,e.created_at])?;
    Ok(())
}

trait Optional<T> {
    fn optional(self) -> SqlResult<Option<T>>;
}
impl<T> Optional<T> for SqlResult<T> {
    fn optional(self) -> SqlResult<Option<T>> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    fn migrations() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
    }
    fn setup() -> (Repository, tempfile::TempDir) {
        let d = tempdir().unwrap();
        let mut c = Connection::open(d.path().join("db.sqlite")).unwrap();
        configure(&c).unwrap();
        run_migrations(&mut c, migrations()).unwrap();
        c.execute("INSERT INTO users(id,created_at) VALUES ('u',1)", [])
            .unwrap();
        c.execute("INSERT INTO tables(id,owner_user_id,room_code,mode,status,created_at,updated_at) VALUES ('t','u','ROOM','classic','playing',1,1)",[]).unwrap();
        (Repository::new(c).unwrap(), d)
    }
    fn state() -> MatchState {
        MatchState {
            id: "m".into(),
            table_id: "t".into(),
            game_type: "durak".into(),
            rules_version: "1".into(),
            mode: "classic".into(),
            status: "active".into(),
            state_json: r#"{"turn":1}"#.into(),
            seq: 1,
            created_at: 1,
            updated_at: 1,
            finished_at: None,
        }
    }
    fn event() -> GameEvent {
        GameEvent {
            id: "e".into(),
            match_id: "m".into(),
            seq: 1,
            public_json: r#"{"action":"start"}"#.into(),
            created_at: 1,
        }
    }
    #[test]
    fn configures_wal_foreign_keys_and_busy_timeout() {
        let d = tempdir().unwrap();
        let c = Connection::open(d.path().join("x")).unwrap();
        configure(&c).unwrap();
        assert_eq!(
            c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            c.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        assert_eq!(
            c.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            5000
        );
    }
    #[test]
    fn saves_bundle_atomically_and_replays_idempotently() {
        let (mut r, _) = setup();
        let w = MatchWrite {
            state: state(),
            event: Some(event()),
            ledger: vec![],
        };
        r.save_match(&w).unwrap();
        r.save_match(&w).unwrap();
        assert_eq!(
            r.connection()
                .query_row("SELECT COUNT(*) FROM game_events", [], |x| x
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn conflicting_event_rolls_back_match_update() {
        let (mut r, _) = setup();
        let w = MatchWrite {
            state: state(),
            event: Some(event()),
            ledger: vec![],
        };
        r.save_match(&w).unwrap();
        let mut changed = state();
        changed.seq = 2;
        changed.state_json = "changed".into();
        let mut bad = event();
        bad.public_json = "different".into();
        assert!(matches!(
            r.save_match(&MatchWrite {
                state: changed,
                event: Some(bad),
                ledger: vec![]
            }),
            Err(StorageError::IdempotencyConflict { kind: "event", .. })
        ));
        assert_eq!(
            r.connection()
                .query_row("SELECT seq FROM matches WHERE id='m'", [], |x| x
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
    }
    #[test]
    fn failed_migration_is_rolled_back() {
        let d = tempdir().unwrap();
        fs::write(
            d.path().join("0001_bad.sql"),
            "CREATE TABLE okay(id INTEGER); SELECT no_such_function();",
        )
        .unwrap();
        let mut c = Connection::open_in_memory().unwrap();
        assert!(run_migrations(&mut c, d.path()).is_err());
        assert_eq!(
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='okay'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
            0
        );
    }
}
