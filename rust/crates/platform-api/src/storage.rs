use rusqlite::{Connection, Result as SqlResult, TransactionBehavior};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

mod accounts;
mod identity;
mod migrations;
mod reads;
mod transitions;
pub use migrations::{configure, open_and_migrate, run_migrations, AppliedMigration};

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

    pub(crate) fn delete_transition(&mut self, game_id: &str) -> SqlResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM durable_game_transitions WHERE game_id=?1",
            [game_id],
        )?;
        tx.execute(
            "DELETE FROM durable_game_states WHERE game_id=?1",
            [game_id],
        )?;
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
mod tests;
