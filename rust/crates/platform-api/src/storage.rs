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

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DurableAccount {
    pub id: String,
    pub name: String,
    pub balance: i64,
    pub games: i64,
    pub wins: i64,
    pub losses: i64,
    pub avatar_url: Option<String>,
    pub password_hash: Option<String>,
    #[serde(default)]
    pub created_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AccountChange {
    pub before: Option<DurableAccount>,
    pub after: DurableAccount,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DurableTransition {
    pub game_id: String,
    pub expected_seq: Option<i64>,
    pub seq: i64,
    pub state_json: String,
    pub accounts: Vec<AccountChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FinishedGameHistory {
    pub room: String,
    pub mode: String,
    pub at: i64,
    pub loser: String,
    pub bet: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionState {
    pub game_id: String,
    pub seq: i64,
    pub state_json: String,
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

    pub fn authenticate_legacy(&mut self, id: &str, digest: &str) -> SqlResult<bool> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !id.starts_with("discord:") {
            tx.execute("INSERT OR IGNORE INTO legacy_identity(client_id, token_hash) VALUES (?1, ?2)", params![id, digest])?;
        }
        let stored: Option<String> = tx.query_row("SELECT token_hash FROM legacy_identity WHERE client_id=?1", [id], |r| r.get(0)).optional()?;
        tx.commit()?;
        Ok(stored.as_deref() == Some(digest))
    }

    pub fn authenticate_verified(&self, id: &str, digest: &str) -> SqlResult<bool> {
        if !id.starts_with("discord:") {
            return Ok(false);
        }
        let stored: Option<String> = self.connection.query_row("SELECT token_hash FROM legacy_identity WHERE client_id=?1 AND EXISTS (SELECT 1 FROM oauth_identities WHERE provider='discord' AND client_id=?1)", [id], |r| r.get(0)).optional()?;
        Ok(stored.as_deref() == Some(digest))
    }

    pub(crate) fn load_account(&self, id: &str) -> SqlResult<Option<crate::game::Account>> {
        self.connection.query_row("SELECT client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at FROM legacy_accounts WHERE client_id=?1", [id], |r| Ok(crate::game::Account { id:r.get(0)?, name:r.get(1)?, balance:r.get(2)?, games:r.get(3)?, wins:r.get(4)?, losses:r.get(5)?, avatar_url:r.get(6)?, password_hash:r.get(7)?, created_at:r.get(8)? })).optional()
    }

    pub(crate) fn list_accounts(&self) -> SqlResult<Vec<crate::game::Account>> {
        let mut statement = self.connection.prepare("SELECT client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at FROM legacy_accounts")?;
        let rows = statement.query_map([], |r| Ok(crate::game::Account { id:r.get(0)?, name:r.get(1)?, balance:r.get(2)?, games:r.get(3)?, wins:r.get(4)?, losses:r.get(5)?, avatar_url:r.get(6)?, password_hash:r.get(7)?, created_at:r.get(8)? }))?;
        rows.collect()
    }

    pub(crate) fn save_account(&self, account: &crate::game::Account) -> SqlResult<()> {
        self.connection.execute("INSERT INTO legacy_accounts(client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(client_id) DO UPDATE SET name=excluded.name,balance=excluded.balance,games=excluded.games,wins=excluded.wins,losses=excluded.losses,avatar_url=excluded.avatar_url,password_hash=COALESCE(excluded.password_hash, legacy_accounts.password_hash),created_at=COALESCE(legacy_accounts.created_at, excluded.created_at)", params![account.id,account.name,account.balance,account.games,account.wins,account.losses,account.avatar_url,account.password_hash,account.created_at])?;
        Ok(())
    }

    pub(crate) fn insert_temp_account(&mut self, account: &crate::game::Account, token_hash: &str) -> SqlResult<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO legacy_accounts(client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![account.id,account.name,account.balance,account.games,account.wins,account.losses,account.avatar_url,account.password_hash,account.created_at])?;
        tx.execute("INSERT INTO legacy_identity(client_id,token_hash) VALUES (?1,?2)", params![account.id, token_hash])?;
        tx.commit()
    }

    pub(crate) fn delete_account(&mut self, id: &str) -> StorageResult<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM legacy_identity WHERE client_id=?1", [id])?;
        tx.execute("DELETE FROM legacy_accounts WHERE client_id=?1", [id])?;
        let pattern = format!("%\"{id}\"%");
        let mut statement = tx.prepare("SELECT game_id FROM durable_game_states WHERE state_json LIKE ?1")?;
        let games: Vec<String> = statement.query_map([&pattern], |r| r.get(0))?.collect::<SqlResult<_>>()?;
        drop(statement);
        for game in games {
            tx.execute("DELETE FROM lobby_history WHERE game_id=?1", [&game])?;
            tx.execute("DELETE FROM durable_game_transitions WHERE game_id=?1", [&game])?;
            tx.execute("DELETE FROM durable_game_states WHERE game_id=?1", [&game])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn finish_temp_login(&mut self, id: &str, token_hash: &str, password_hash: &str) -> StorageResult<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO legacy_accounts(client_id,name,balance,games,wins,losses,created_at) VALUES (?1,'',1000,0,0,0,unixepoch()) ON CONFLICT(client_id) DO NOTHING", [id])?;
        tx.execute("UPDATE legacy_accounts SET password_hash=?2, created_at=COALESCE(created_at, unixepoch()) WHERE client_id=?1", params![id, password_hash])?;
        tx.execute("INSERT INTO legacy_identity(client_id,token_hash) VALUES (?1,?2) ON CONFLICT(client_id) DO UPDATE SET token_hash=excluded.token_hash", params![id, token_hash])?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn replace_password(&mut self, id: &str, password_hash: &str, revoke_session: bool) -> StorageResult<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("UPDATE legacy_accounts SET password_hash=?2 WHERE client_id=?1", params![id, password_hash])?;
        if revoke_session {
            tx.execute("DELETE FROM legacy_identity WHERE client_id=?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn oauth_login(&mut self, subject: &str, candidate_id: &str, digest: &str) -> SqlResult<String> {
        if !candidate_id.starts_with("discord:") {
            return Err(rusqlite::Error::InvalidParameterName("OAuth client id namespace".into()));
        }
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT OR IGNORE INTO oauth_identities(provider,subject,client_id) VALUES ('discord',?1,?2)", params![subject,candidate_id])?;
        let id: String = tx.query_row("SELECT client_id FROM oauth_identities WHERE provider='discord' AND subject=?1", [subject], |r| r.get(0))?;
        tx.execute("INSERT INTO legacy_identity(client_id,token_hash) VALUES (?1,?2) ON CONFLICT(client_id) DO UPDATE SET token_hash=excluded.token_hash", params![id,digest])?;
        tx.commit()?;
        Ok(id)
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

    pub fn commit_transition(&mut self, transition: &DurableTransition) -> StorageResult<()> {
        self.commit_transition_with_history(transition, None)
    }

    pub fn commit_transition_with_history(
        &mut self,
        transition: &DurableTransition,
        history: Option<&FinishedGameHistory>,
    ) -> StorageResult<()> {
        if transition.game_id.is_empty() || transition.seq < 0 || transition.state_json.is_empty()
            || transition.expected_seq.is_some_and(|seq| seq.checked_add(1) != Some(transition.seq))
            || (transition.expected_seq.is_none() && transition.seq != 0)
        {
            return Err(StorageError::IdempotencyConflict { kind: "transition sequence", key: transition.game_id.clone() });
        }
        let payload = serde_json::to_string(transition).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx.query_row("SELECT payload_json FROM durable_game_transitions WHERE game_id=?1 AND seq=?2", params![transition.game_id, transition.seq], |r| r.get(0)).optional()?;
        if let Some(existing) = existing {
            let stored_history: Option<FinishedGameHistory> = tx.query_row("SELECT room,mode,at,loser,bet FROM lobby_history WHERE game_id=?1 AND seq=?2", params![transition.game_id, transition.seq], |row| Ok(FinishedGameHistory { room: row.get(0)?, mode: row.get(1)?, at: row.get(2)?, loser: row.get(3)?, bet: row.get(4)? })).optional()?;
            return if existing == payload && stored_history.as_ref() == history { Ok(()) } else { Err(StorageError::IdempotencyConflict { kind: "transition", key: transition.game_id.clone() }) };
        }
        let current: Option<i64> = tx.query_row("SELECT seq FROM durable_game_states WHERE game_id=?1", [&transition.game_id], |r| r.get(0)).optional()?;
        if current != transition.expected_seq {
            return Err(StorageError::IdempotencyConflict { kind: "transition sequence", key: transition.game_id.clone() });
        }
        let mut seen = std::collections::HashSet::new();
        for change in &transition.accounts {
            let after = &change.after;
            if !seen.insert(&after.id) {
                return Err(StorageError::IdempotencyConflict { kind: "account", key: after.id.clone() });
            }
            let count = if let Some(before) = &change.before {
                if before.id != after.id {
                    return Err(StorageError::IdempotencyConflict { kind: "account", key: after.id.clone() });
                }
                tx.execute("UPDATE legacy_accounts SET name=?2,balance=?3,games=?4,wins=?5,losses=?6,avatar_url=?7,password_hash=?8 WHERE client_id=?1 AND name=?9 AND balance=?10 AND games=?11 AND wins=?12 AND losses=?13 AND (avatar_url IS ?14 OR avatar_url=?14) AND (password_hash IS ?15 OR password_hash=?15)", params![after.id,after.name,after.balance,after.games,after.wins,after.losses,after.avatar_url,after.password_hash,before.name,before.balance,before.games,before.wins,before.losses,before.avatar_url,before.password_hash])?
            } else {
                tx.execute("INSERT OR IGNORE INTO legacy_accounts(client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![after.id,after.name,after.balance,after.games,after.wins,after.losses,after.avatar_url,after.password_hash,after.created_at])?
            };
            if count != 1 {
                return Err(StorageError::IdempotencyConflict { kind: "account", key: after.id.clone() });
            }
        }
        tx.execute("INSERT INTO durable_game_states(game_id,seq,state_json) VALUES (?1,?2,?3) ON CONFLICT(game_id) DO UPDATE SET seq=excluded.seq,state_json=excluded.state_json", params![transition.game_id,transition.seq,transition.state_json])?;
        tx.execute("INSERT INTO durable_game_transitions(game_id,seq,payload_json) VALUES (?1,?2,?3)", params![transition.game_id,transition.seq,payload])?;
        if let Some(history) = history {
            insert_history(&tx, &transition.game_id, transition.seq, history)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn list_history(&self, limit: u32) -> SqlResult<Vec<FinishedGameHistory>> {
        let mut statement = self.connection.prepare("SELECT room,mode,at,loser,bet FROM lobby_history ORDER BY ordinal DESC LIMIT ?1")?;
        let rows = statement.query_map([i64::from(limit.min(100))], |row| Ok(FinishedGameHistory { room: row.get(0)?, mode: row.get(1)?, at: row.get(2)?, loser: row.get(3)?, bet: row.get(4)? }))?;
        rows.collect()
    }

    pub fn load_transition_snapshot(&self, game_id: &str) -> SqlResult<Option<TransitionState>> {
        self.connection.query_row(
            "SELECT game_id,seq,state_json FROM durable_game_states WHERE game_id=?1",
            [game_id],
            |row| Ok(TransitionState { game_id: row.get(0)?, seq: row.get(1)?, state_json: row.get(2)? }),
        ).optional()
    }

    pub fn list_transition_snapshots(&self) -> SqlResult<Vec<TransitionState>> {
        let mut statement = self.connection.prepare(
            "SELECT game_id,seq,state_json FROM durable_game_states ORDER BY game_id",
        )?;
        let rows = statement.query_map([], |row| Ok(TransitionState {
            game_id: row.get(0)?, seq: row.get(1)?, state_json: row.get(2)?,
        }))?;
        rows.collect()
    }

    pub fn load_transition_state(&self, game_id: &str) -> SqlResult<Option<(i64, String)>> {
        self.connection.query_row("SELECT seq,state_json FROM durable_game_states WHERE game_id=?1", [game_id], |r| Ok((r.get(0)?,r.get(1)?))).optional()
    }

    pub(crate) fn delete_transition(&mut self, game_id: &str) -> SqlResult<()> {
        let tx = self.connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM durable_game_transitions WHERE game_id=?1", [game_id])?;
        tx.execute("DELETE FROM durable_game_states WHERE game_id=?1", [game_id])?;
        tx.commit()
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

fn insert_history(tx: &Transaction<'_>, game_id: &str, seq: i64, h: &FinishedGameHistory) -> StorageResult<()> {
    if h.room.is_empty() || h.loser.is_empty() || h.at < 0 || h.bet < 0 || !matches!(h.mode.as_str(), "classic" | "transfer") {
        return Err(StorageError::IdempotencyConflict { kind: "history", key: game_id.to_owned() });
    }
    let existing: Option<(String, String, i64, String, i64)> = tx.query_row("SELECT room,mode,at,loser,bet FROM lobby_history WHERE game_id=?1 AND seq=?2", params![game_id, seq], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional()?;
    let expected = (h.room.clone(), h.mode.clone(), h.at, h.loser.clone(), h.bet);
    if let Some(existing) = existing {
        if existing != expected {
            return Err(StorageError::IdempotencyConflict { kind: "history", key: format!("{game_id}:{seq}") });
        }
        return Ok(());
    }
    tx.execute("INSERT INTO lobby_history(game_id,seq,room,mode,at,loser,bet) VALUES (?1,?2,?3,?4,?5,?6,?7)", params![game_id,seq,h.room,h.mode,h.at,h.loser,h.bet])?;
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
    fn durable_transition_is_atomic_idempotent_and_survives_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("transition.sqlite");
        let mut repo = Repository::open(&path, migrations()).unwrap();
        let account = crate::game::Account::new("a", "Alice");
        repo.save_account(&account).unwrap();
        let before = DurableAccount { id: "a".into(), name: "Alice".into(), balance: 1000, games: 0, wins: 0, losses: 0, avatar_url: None, password_hash: None, created_at: None };
        let mut after = before.clone();
        after.balance = 900;
        let first = DurableTransition { game_id: "game".into(), expected_seq: None, seq: 0, state_json: "waiting".into(), accounts: vec![] };
        repo.commit_transition(&first).unwrap();
        let next = DurableTransition { game_id: "game".into(), expected_seq: Some(0), seq: 1, state_json: "playing".into(), accounts: vec![AccountChange { before: Some(before.clone()), after: after.clone() }] };
        repo.commit_transition(&next).unwrap();
        repo.commit_transition(&next).unwrap();
        let mut conflict = next.clone();
        conflict.state_json = "different".into();
        assert!(repo.commit_transition(&conflict).is_err());
        let mut invalid = next.clone();
        invalid.expected_seq = Some(1);
        invalid.seq = 2;
        invalid.accounts[0].before = Some(after.clone());
        invalid.accounts[0].after.balance = -1;
        assert!(repo.commit_transition(&invalid).is_err());
        assert_eq!(repo.load_transition_state("game").unwrap(), Some((1, "playing".into())));
        assert_eq!(repo.load_account("a").unwrap().unwrap().balance, 900);
        drop(repo);
        let repo = Repository::open(&path, migrations()).unwrap();
        assert_eq!(repo.load_transition_state("game").unwrap(), Some((1, "playing".into())));
        assert_eq!(repo.load_account("a").unwrap().unwrap().balance, 900);
    }
    #[test]
    fn finished_history_is_atomic_idempotent_and_restored_without_users() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let history = FinishedGameHistory { room: "Комната".into(), mode: "classic".into(), at: 42, loser: "discord:loser".into(), bet: 100 };
        {
            let mut repo = Repository::open(&path, migrations()).unwrap();
            let transition = DurableTransition { game_id: "game".into(), expected_seq: None, seq: 0, state_json: "finished".into(), accounts: vec![] };
            repo.commit_transition_with_history(&transition, Some(&history)).unwrap();
            repo.commit_transition_with_history(&transition, Some(&history)).unwrap();
            assert_eq!(repo.list_history(8).unwrap(), vec![history.clone()]);
            assert_eq!(repo.connection().query_row("SELECT COUNT(*) FROM legacy_accounts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
            let mut changed = history.clone();
            changed.bet = 200;
            assert!(repo.commit_transition_with_history(&transition, Some(&changed)).is_err());
        }
        let repo = Repository::open(&path, migrations()).unwrap();
        assert_eq!(repo.list_history(8).unwrap(), vec![history]);
    }

    #[test]
    fn invalid_history_rolls_back_transition() {
        let dir = tempdir().unwrap();
        let mut repo = Repository::open(dir.path().join("history-failure.sqlite"), migrations()).unwrap();
        let transition = DurableTransition { game_id: "game".into(), expected_seq: None, seq: 0, state_json: "finished".into(), accounts: vec![] };
        let invalid = FinishedGameHistory { room: "room".into(), mode: "unknown".into(), at: 1, loser: "discord:x".into(), bet: 0 };
        assert!(repo.commit_transition_with_history(&transition, Some(&invalid)).is_err());
        assert!(repo.load_transition_state("game").unwrap().is_none());
        assert!(repo.list_history(8).unwrap().is_empty());
    }

    #[test]
    fn history_lists_newest_first_for_reused_room_and_limits_results() {
        let dir = tempdir().unwrap();
        let mut repo = Repository::open(dir.path().join("ordering.sqlite"), migrations()).unwrap();
        for seq in 0..10 {
            let transition = DurableTransition { game_id: "game".into(), expected_seq: if seq == 0 { None } else { Some(seq - 1) }, seq, state_json: format!("state-{seq}"), accounts: vec![] };
            let history = FinishedGameHistory { room: "room".into(), mode: "transfer".into(), at: 1, loser: format!("player-{seq}"), bet: 0 };
            repo.commit_transition_with_history(&transition, Some(&history)).unwrap();
        }
        let entries = repo.list_history(8).unwrap();
        assert_eq!(entries.len(), 8);
        assert_eq!(entries[0].loser, "player-9");
        assert_eq!(entries[7].loser, "player-2");
        assert!(repo.list_history(0).unwrap().is_empty());
    }

    #[test]
    fn transition_snapshots_list_latest_states_after_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("snapshots.sqlite");
        let mut repo = Repository::open(&path, migrations()).unwrap();
        assert!(repo.list_transition_snapshots().unwrap().is_empty());
        assert_eq!(repo.load_transition_snapshot("missing").unwrap(), None);
        for (game_id, expected_seq, seq, state_json) in [
            ("z", None, 0, "{\"status\":\"waiting\"}"),
            ("a", None, 0, "{\"status\":\"waiting\"}"),
            ("z", Some(0), 1, "{\"status\":\"playing\"}"),
        ] {
            repo.commit_transition(&DurableTransition {
                game_id: game_id.into(), expected_seq, seq, state_json: state_json.into(), accounts: vec![],
            }).unwrap();
        }
        drop(repo);
        let repo = Repository::open(&path, migrations()).unwrap();
        let expected = vec![
            TransitionState { game_id: "a".into(), seq: 0, state_json: "{\"status\":\"waiting\"}".into() },
            TransitionState { game_id: "z".into(), seq: 1, state_json: "{\"status\":\"playing\"}".into() },
        ];
        assert_eq!(repo.list_transition_snapshots().unwrap(), expected);
        assert_eq!(repo.load_transition_snapshot("z").unwrap(), Some(expected[1].clone()));
        assert_eq!(repo.load_transition_snapshot("missing").unwrap(), None);
    }

    #[test]
    fn verified_authentication_requires_existing_discord_identity() {
        let dir = tempdir().unwrap();
        let mut repo = Repository::open(dir.path().join("verified.sqlite"), migrations()).unwrap();
        assert!(!repo.authenticate_verified("guest", "token").unwrap());
        assert!(!repo.authenticate_verified("discord:fake", "token").unwrap());
        repo.authenticate_legacy("guest", "token").unwrap();
        assert!(!repo.authenticate_verified("guest", "token").unwrap());
        repo.oauth_login("123", "discord:real", "token").unwrap();
        assert!(repo.authenticate_verified("discord:real", "token").unwrap());
        assert!(!repo.authenticate_verified("discord:real", "wrong").unwrap());
        assert_eq!(repo.connection().query_row("SELECT COUNT(*) FROM legacy_accounts", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn oauth_identity_and_account_survive_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("oauth.sqlite");
        {
            let mut repo = Repository::open(&path, migrations()).unwrap();
            assert!(!repo.authenticate_legacy("discord:first", "attacker").unwrap());
            assert_eq!(repo.oauth_login("123", "discord:first", "hash1").unwrap(), "discord:first");
            let mut account = crate::game::Account::new("discord:first", "Alice");
            account.balance = 1250;
            repo.save_account(&account).unwrap();
        }
        let mut repo = Repository::open(&path, migrations()).unwrap();
        assert_eq!(repo.oauth_login("123", "discord:second", "hash2").unwrap(), "discord:first");
        assert!(!repo.authenticate_legacy("discord:first", "hash1").unwrap());
        assert!(repo.authenticate_legacy("discord:first", "hash2").unwrap());
        let account = repo.load_account("discord:first").unwrap().unwrap();
        assert_eq!(account.name, "Alice");
        assert_eq!(account.balance, 1250);
    }
    #[test]
    fn deleting_account_removes_identity_credentials_and_its_finished_games() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("delete.sqlite");
        let mut repo = Repository::open(&path, migrations()).unwrap();
        let mut account = crate::game::Account::new("temp:gone", "Гость");
        account.password_hash = Some("pbkdf2_sha256$1$aa$bb".into());
        repo.insert_temp_account(&account, "hash").unwrap();
        repo.save_account(&account).unwrap();
        let history = FinishedGameHistory { room: "Стол".into(), mode: "classic".into(), at: 1, loser: "Гость".into(), bet: 0 };
        let transition = DurableTransition { game_id: "ROOM01".into(), expected_seq: None, seq: 0, state_json: "{\"id\":\"ROOM01\",\"state\":{\"players\":[{\"id\":\"temp:gone\"}]}}".into(), accounts: Vec::new() };
        repo.commit_transition_with_history(&transition, Some(&history)).unwrap();
        repo.delete_account("temp:gone").unwrap();
        assert!(repo.load_account("temp:gone").unwrap().is_none());
        assert_eq!(repo.authenticate_verified("temp:gone", "hash").unwrap(), false);
        assert!(repo.connection().query_row("SELECT COUNT(*) FROM legacy_identity WHERE client_id='temp:gone'", [], |r| r.get::<_, i64>(0)).unwrap() == 0);
        assert!(repo.load_transition_state("ROOM01").unwrap().is_none());
        assert!(repo.list_history(8).unwrap().is_empty());
        assert!(repo.delete_account("temp:gone").is_ok());
    }
    #[test]
    fn finish_temp_login_replaces_password_without_duplicating_account() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("login.sqlite");
        let mut repo = Repository::open(&path, migrations()).unwrap();
        let mut account = crate::game::Account::new("temp:first", "Гость");
        account.balance = 420;
        repo.insert_temp_account(&account, "pending").unwrap();
        repo.finish_temp_login("temp:first", "token", "pbkdf2_sha256$1$aa$bb").unwrap();
        let stored = repo.load_account("temp:first").unwrap().unwrap();
        assert_eq!(stored.balance, 420);
        assert_eq!(stored.password_hash.as_deref(), Some("pbkdf2_sha256$1$aa$bb"));
        assert!(repo.authenticate_legacy("temp:first", "token").unwrap());
        repo.replace_password("temp:first", "new", true).unwrap();
        assert_eq!(repo.load_account("temp:first").unwrap().unwrap().password_hash.as_deref(), Some("new"));
        assert!(repo.connection().query_row("SELECT COUNT(*) FROM legacy_identity WHERE client_id='temp:first'", [], |r| r.get::<_, i64>(0)).unwrap() == 0);
    }
    #[test]
    fn legacy_binding_survives_reopen_and_rejects_other_tokens() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("identity.sqlite");
        {
            let mut repo = Repository::open(&path, migrations()).unwrap();
            assert!(repo.authenticate_legacy("client", "digest-a").unwrap());
            assert!(!repo.authenticate_legacy("client", "digest-b").unwrap());
        }
        let mut repo = Repository::open(&path, migrations()).unwrap();
        assert!(repo.authenticate_legacy("client", "digest-a").unwrap());
        assert!(!repo.authenticate_legacy("client", "digest-b").unwrap());
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
