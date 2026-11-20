use super::Repository;
use rusqlite::{params, OptionalExtension, Result as SqlResult, TransactionBehavior};

impl Repository {
    pub fn authenticate_legacy(&mut self, id: &str, digest: &str) -> SqlResult<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !id.starts_with("discord:") {
            tx.execute(
                "INSERT OR IGNORE INTO legacy_identity(client_id, token_hash) VALUES (?1, ?2)",
                params![id, digest],
            )?;
        }
        let stored: Option<String> = tx
            .query_row(
                "SELECT token_hash FROM legacy_identity WHERE client_id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?;
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

    pub(crate) fn oauth_login(
        &mut self,
        subject: &str,
        candidate_id: &str,
        digest: &str,
    ) -> SqlResult<String> {
        if !candidate_id.starts_with("discord:") {
            return Err(rusqlite::Error::InvalidParameterName(
                "OAuth client id namespace".into(),
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("INSERT OR IGNORE INTO oauth_identities(provider,subject,client_id) VALUES ('discord',?1,?2)", params![subject,candidate_id])?;
        let id: String = tx.query_row(
            "SELECT client_id FROM oauth_identities WHERE provider='discord' AND subject=?1",
            [subject],
            |r| r.get(0),
        )?;
        tx.execute("INSERT INTO legacy_identity(client_id,token_hash) VALUES (?1,?2) ON CONFLICT(client_id) DO UPDATE SET token_hash=excluded.token_hash", params![id,digest])?;
        tx.commit()?;
        Ok(id)
    }
}
