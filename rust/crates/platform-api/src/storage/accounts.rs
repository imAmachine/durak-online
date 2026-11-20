use super::{Optional, Repository, StorageResult};
use rusqlite::{params, Result as SqlResult, TransactionBehavior};

fn account_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::game::Account> {
    Ok(crate::game::Account {
        id: row.get(0)?,
        name: row.get(1)?,
        balance: row.get(2)?,
        games: row.get(3)?,
        wins: row.get(4)?,
        losses: row.get(5)?,
        avatar_url: row.get(6)?,
        password_hash: row.get(7)?,
        created_at: row.get(8)?,
    })
}

impl Repository {
    pub(crate) fn load_account(&self, id: &str) -> SqlResult<Option<crate::game::Account>> {
        self.connection
            .query_row(
                "SELECT client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at FROM legacy_accounts WHERE client_id=?1",
                [id],
                account_from_row,
            )
            .optional()
    }

    pub(crate) fn list_accounts(&self) -> SqlResult<Vec<crate::game::Account>> {
        let mut statement = self.connection.prepare("SELECT client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at FROM legacy_accounts")?;
        let rows = statement.query_map([], account_from_row)?;
        rows.collect()
    }

    pub(crate) fn save_account(&self, account: &crate::game::Account) -> SqlResult<()> {
        self.connection.execute("INSERT INTO legacy_accounts(client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(client_id) DO UPDATE SET name=excluded.name,balance=excluded.balance,games=excluded.games,wins=excluded.wins,losses=excluded.losses,avatar_url=excluded.avatar_url,password_hash=COALESCE(excluded.password_hash, legacy_accounts.password_hash),created_at=COALESCE(legacy_accounts.created_at, excluded.created_at)", params![account.id,account.name,account.balance,account.games,account.wins,account.losses,account.avatar_url,account.password_hash,account.created_at])?;
        Ok(())
    }

    pub(crate) fn insert_temp_account(
        &mut self,
        account: &crate::game::Account,
        token_hash: &str,
    ) -> SqlResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO legacy_accounts(client_id,name,balance,games,wins,losses,avatar_url,password_hash,created_at) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![account.id,account.name,account.balance,account.games,account.wins,account.losses,account.avatar_url,account.password_hash,account.created_at])?;
        tx.execute(
            "INSERT INTO legacy_identity(client_id,token_hash) VALUES (?1,?2)",
            params![account.id, token_hash],
        )?;
        tx.commit()
    }

    pub(crate) fn delete_account(
        &mut self,
        id: &str,
        live_rooms: &std::collections::HashSet<String>,
    ) -> StorageResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM legacy_identity WHERE client_id=?1", [id])?;
        tx.execute("DELETE FROM legacy_accounts WHERE client_id=?1", [id])?;
        let pattern = format!("%\"{id}\"%");
        let mut statement =
            tx.prepare("SELECT game_id FROM durable_game_states WHERE state_json LIKE ?1")?;
        let games: Vec<String> = statement
            .query_map([&pattern], |r| r.get(0))?
            .collect::<SqlResult<_>>()?;
        drop(statement);
        for game in games {
            if live_rooms.contains(&game) {
                continue;
            }
            tx.execute("DELETE FROM lobby_history WHERE game_id=?1", [&game])?;
            tx.execute(
                "DELETE FROM durable_game_transitions WHERE game_id=?1",
                [&game],
            )?;
            tx.execute("DELETE FROM durable_game_states WHERE game_id=?1", [&game])?;
        }
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn finish_temp_login(
        &mut self,
        id: &str,
        token_hash: &str,
        password_hash: &str,
    ) -> StorageResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT INTO legacy_accounts(client_id,name,balance,games,wins,losses,created_at) VALUES (?1,'',1000,0,0,0,unixepoch()) ON CONFLICT(client_id) DO NOTHING", [id])?;
        tx.execute("UPDATE legacy_accounts SET password_hash=?2, created_at=COALESCE(created_at, unixepoch()) WHERE client_id=?1", params![id, password_hash])?;
        tx.execute("INSERT INTO legacy_identity(client_id,token_hash) VALUES (?1,?2) ON CONFLICT(client_id) DO UPDATE SET token_hash=excluded.token_hash", params![id, token_hash])?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn replace_password(
        &mut self,
        id: &str,
        password_hash: &str,
        revoke_session: bool,
    ) -> StorageResult<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE legacy_accounts SET password_hash=?2 WHERE client_id=?1",
            params![id, password_hash],
        )?;
        if revoke_session {
            tx.execute("DELETE FROM legacy_identity WHERE client_id=?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }
}
