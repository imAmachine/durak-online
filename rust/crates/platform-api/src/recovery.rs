use super::{game, seeded_test_identity, storage, AppState};
#[cfg(test)]
use super::{oauth, TEST_IDENTITIES};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};

impl AppState {
    pub fn with_repository(
        repository: storage::Repository,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        #[cfg(test)]
        let mut repository = repository;
        #[cfg(test)]
        {
            let token = oauth::digest(&"a".repeat(64));
            for id in TEST_IDENTITIES {
                let _ = repository.oauth_login(id, &format!("discord:{id}"), &token)?;
            }
        }
        let mut rooms = HashMap::new();
        let mut accounts = repository
            .list_accounts()?
            .into_iter()
            .map(|account| (account.id.clone(), account))
            .collect::<HashMap<_, _>>();
        let mut seated = std::collections::HashSet::new();
        for saved in repository.list_transition_snapshots()? {
            let room: game::Room = serde_json::from_str(&saved.state_json)?;
            if room.state.status == durak_engine::Status::Playing {
                for player in room.state.players.iter().filter(|p| !p.left) {
                    let linked: bool = if seeded_test_identity(&player.id) {
                        true
                    } else if player.id.starts_with("discord:") {
                        repository.connection().query_row(
                            "SELECT EXISTS (SELECT 1 FROM oauth_identities WHERE provider='discord' AND client_id=?1)",
                            [&player.id], |row| row.get(0))?
                    } else if let Some(stored) = player
                        .id
                        .strip_prefix("temp:")
                        .filter(|rest| !rest.is_empty())
                    {
                        let stored_id = format!("temp:{stored}");
                        repository.connection().query_row(
                            "SELECT EXISTS (SELECT 1 FROM legacy_accounts WHERE client_id=?1 AND password_hash IS NOT NULL)",
                            [&stored_id], |row| row.get(0))?
                    } else {
                        false
                    };
                    if !linked {
                        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("active room {} contains an unlinked player {}; administrator must migrate or cancel this room before startup", room.id, player.id)).into());
                    }
                }
            }
            let invalid = || {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid room snapshot {}", saved.game_id),
                )
            };
            if room.id != saved.game_id
                || room.seq.checked_sub(1).and_then(|n| i64::try_from(n).ok()) != Some(saved.seq)
                || room
                    .events
                    .last()
                    .and_then(|e| e.get("seq"))
                    .and_then(|s| s.as_u64())
                    != Some(room.seq)
                || room.state.players.len() > 6
                || room.bet < 0
                || room.pot < 0
                || room.mode != room.state.mode
                || room.deck_size != room.state.deck_size
            {
                return Err(invalid().into());
            }
            let mut players = std::collections::HashSet::new();
            for player in &room.state.players {
                if !players.insert(player.id.clone())
                    || (!player.left && !seated.insert(player.id.clone()))
                {
                    return Err(invalid().into());
                }
                if !accounts.contains_key(&player.id) {
                    let account = repository.load_account(&player.id)?.ok_or_else(invalid)?;
                    accounts.insert(player.id.clone(), account);
                }
            }
            if !room.state.players.is_empty() {
                if !players.contains(&room.host) {
                    return Err(invalid().into());
                }
                rooms.insert(room.id.clone(), room);
            }
        }
        Ok(Self {
            rooms: Arc::new(RwLock::new(rooms)),
            accounts: Arc::new(RwLock::new(accounts)),
            offline_rooms: Arc::new(RwLock::new(HashMap::new())),
            repository: Some(Arc::new(Mutex::new(repository))),
            ready: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            ..Self::default()
        })
    }
}
