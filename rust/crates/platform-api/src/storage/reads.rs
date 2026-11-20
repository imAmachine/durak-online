use super::{FinishedGameHistory, Optional, Repository, TransitionState};
use rusqlite::Result as SqlResult;

impl Repository {
    pub fn list_history(&self, limit: u32) -> SqlResult<Vec<FinishedGameHistory>> {
        let mut statement = self.connection.prepare(
            "SELECT room,mode,at,loser,bet FROM lobby_history ORDER BY ordinal DESC LIMIT ?1",
        )?;
        let rows = statement.query_map([i64::from(limit.min(100))], |row| {
            Ok(FinishedGameHistory {
                room: row.get(0)?,
                mode: row.get(1)?,
                at: row.get(2)?,
                loser: row.get(3)?,
                bet: row.get(4)?,
            })
        })?;
        rows.collect()
    }

    pub fn load_transition_snapshot(&self, game_id: &str) -> SqlResult<Option<TransitionState>> {
        self.connection
            .query_row(
                "SELECT game_id,seq,state_json FROM durable_game_states WHERE game_id=?1",
                [game_id],
                |row| {
                    Ok(TransitionState {
                        game_id: row.get(0)?,
                        seq: row.get(1)?,
                        state_json: row.get(2)?,
                    })
                },
            )
            .optional()
    }

    pub fn list_transition_snapshots(&self) -> SqlResult<Vec<TransitionState>> {
        let mut statement = self
            .connection
            .prepare("SELECT game_id,seq,state_json FROM durable_game_states ORDER BY game_id")?;
        let rows = statement.query_map([], |row| {
            Ok(TransitionState {
                game_id: row.get(0)?,
                seq: row.get(1)?,
                state_json: row.get(2)?,
            })
        })?;
        rows.collect()
    }

    pub fn load_transition_state(&self, game_id: &str) -> SqlResult<Option<(i64, String)>> {
        self.connection
            .query_row(
                "SELECT seq,state_json FROM durable_game_states WHERE game_id=?1",
                [game_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
    }
}
