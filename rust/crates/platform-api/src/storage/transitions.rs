use super::{
    DurableTransition, FinishedGameHistory, GameEvent, LedgerEntry, MatchState, MatchWrite,
    Optional, Repository, StorageError, StorageResult,
};
use rusqlite::{params, Transaction, TransactionBehavior};

impl Repository {
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
        if transition.game_id.is_empty()
            || transition.seq < 0
            || transition.state_json.is_empty()
            || transition
                .expected_seq
                .is_some_and(|seq| seq.checked_add(1) != Some(transition.seq))
            || (transition.expected_seq.is_none() && transition.seq != 0)
        {
            return Err(StorageError::IdempotencyConflict {
                kind: "transition sequence",
                key: transition.game_id.clone(),
            });
        }
        let payload = serde_json::to_string(transition)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row(
                "SELECT payload_json FROM durable_game_transitions WHERE game_id=?1 AND seq=?2",
                params![transition.game_id, transition.seq],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            let stored_history: Option<FinishedGameHistory> = tx
                .query_row(
                    "SELECT room,mode,at,loser,bet FROM lobby_history WHERE game_id=?1 AND seq=?2",
                    params![transition.game_id, transition.seq],
                    |row| {
                        Ok(FinishedGameHistory {
                            room: row.get(0)?,
                            mode: row.get(1)?,
                            at: row.get(2)?,
                            loser: row.get(3)?,
                            bet: row.get(4)?,
                        })
                    },
                )
                .optional()?;
            return if existing == payload && stored_history.as_ref() == history {
                Ok(())
            } else {
                Err(StorageError::IdempotencyConflict {
                    kind: "transition",
                    key: transition.game_id.clone(),
                })
            };
        }
        let current: Option<i64> = tx
            .query_row(
                "SELECT seq FROM durable_game_states WHERE game_id=?1",
                [&transition.game_id],
                |r| r.get(0),
            )
            .optional()?;
        if current != transition.expected_seq {
            return Err(StorageError::IdempotencyConflict {
                kind: "transition sequence",
                key: transition.game_id.clone(),
            });
        }
        let mut seen = std::collections::HashSet::new();
        for change in &transition.accounts {
            let after = &change.after;
            if !seen.insert(&after.id) {
                return Err(StorageError::IdempotencyConflict {
                    kind: "account",
                    key: after.id.clone(),
                });
            }
            let count = if let Some(before) = &change.before {
                if before.id != after.id {
                    return Err(StorageError::IdempotencyConflict {
                        kind: "account",
                        key: after.id.clone(),
                    });
                }
                tx.execute("UPDATE legacy_accounts SET name=?2,balance=?3,games=?4,wins=?5,losses=?6,avatar_url=?7,password_hash=?8 WHERE client_id=?1 AND name=?9 AND balance=?10 AND games=?11 AND wins=?12 AND losses=?13 AND (avatar_url IS ?14 OR avatar_url=?14) AND (password_hash IS ?15 OR password_hash=?15)", params![after.id,after.name,after.balance,after.games,after.wins,after.losses,after.avatar_url,after.password_hash,before.name,before.balance,before.games,before.wins,before.losses,before.avatar_url,before.password_hash])?
            } else {
                tx.execute("INSERT OR IGNORE INTO legacy_accounts(client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![after.id,after.name,after.balance,after.games,after.wins,after.losses,after.avatar_url,after.password_hash,after.created_at])?
            };
            if count != 1 {
                return Err(StorageError::IdempotencyConflict {
                    kind: "account",
                    key: after.id.clone(),
                });
            }
        }
        tx.execute("INSERT INTO durable_game_states(game_id,seq,state_json) VALUES (?1,?2,?3) ON CONFLICT(game_id) DO UPDATE SET seq=excluded.seq,state_json=excluded.state_json", params![transition.game_id,transition.seq,transition.state_json])?;
        tx.execute(
            "INSERT INTO durable_game_transitions(game_id,seq,payload_json) VALUES (?1,?2,?3)",
            params![transition.game_id, transition.seq, payload],
        )?;
        if let Some(history) = history {
            insert_history(&tx, &transition.game_id, transition.seq, history)?;
        }
        tx.commit()?;
        Ok(())
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

fn insert_history(
    tx: &Transaction<'_>,
    game_id: &str,
    seq: i64,
    h: &FinishedGameHistory,
) -> StorageResult<()> {
    if h.room.is_empty()
        || h.loser.is_empty()
        || h.at < 0
        || h.bet < 0
        || !matches!(h.mode.as_str(), "classic" | "transfer")
    {
        return Err(StorageError::IdempotencyConflict {
            kind: "history",
            key: game_id.to_owned(),
        });
    }
    let existing: Option<(String, String, i64, String, i64)> = tx
        .query_row(
            "SELECT room,mode,at,loser,bet FROM lobby_history WHERE game_id=?1 AND seq=?2",
            params![game_id, seq],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let expected = (h.room.clone(), h.mode.clone(), h.at, h.loser.clone(), h.bet);
    if let Some(existing) = existing {
        if existing != expected {
            return Err(StorageError::IdempotencyConflict {
                kind: "history",
                key: format!("{game_id}:{seq}"),
            });
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
