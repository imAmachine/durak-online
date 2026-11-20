mod actions;
mod admin;
mod cards;
mod commit;
mod leave;
mod profile;
mod projection;
mod rooms;

pub use actions::action;
pub use admin::admin;
#[cfg(test)]
use admin::admin_view;
pub use leave::leave;
use leave::leave_inner;
pub use profile::{player_profile, profile};
pub use rooms::{create, join};

use super::{error, is_client_id, is_token, AppState};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
#[cfg(test)]
use cards::card;
use cards::card_json;
use commit::{broadcast, commit_room, publish_room, push_snapshot, CommitMeta};
#[cfg(test)]
use durak_engine::{Action, Command};
#[cfg(test)]
use durak_engine::{Card, Rank, Suit};
use durak_engine::{DeckSize, DurakState, Mode};
use projection::{room_summary, snapshot, status};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{interval, Duration};

const START_BALANCE: i64 = 1000;
const BETS: [i64; 5] = [0, 50, 100, 250, 500];
const EVENT_LIMIT: usize = 40;
const SNAPSHOT_LOG_LIMIT: usize = 8;
const ROOM_NAME_LIMIT: usize = 40;
const OFFLINE_GRACE_MS: u64 = 5 * 60 * 1000;
const FINISHED_ROOM_TTL_MS: u64 = 30 * 60 * 1000;

struct SecureRandom;
impl durak_engine::RandomSource for SecureRandom {
    fn next_usize(&mut self, upper: usize) -> usize {
        let zone = u64::MAX - u64::MAX % upper as u64;
        loop {
            let mut bytes = [0u8; 8];
            getrandom::getrandom(&mut bytes).expect("OS random source unavailable");
            let n = u64::from_ne_bytes(bytes);
            if n < zone {
                return (n % upper as u64) as usize;
            }
        }
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone, PartialEq, Eq)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub balance: i64,
    pub games: i64,
    pub wins: i64,
    pub losses: i64,
    pub avatar_url: Option<String>,
    pub password_hash: Option<String>,
    pub created_at: Option<i64>,
}
impl Account {
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            balance: START_BALANCE,
            games: 0,
            wins: 0,
            losses: 0,
            avatar_url: None,
            password_hash: None,
            created_at: Some(i64::try_from(now() / 1000).unwrap_or(i64::MAX)),
        }
    }
    pub fn public(&self, online: bool) -> Value {
        json!({"id":self.id,"name":self.name,"balance":self.balance,"games":self.games,"wins":self.wins,"losses":self.losses,"avatarUrl":self.avatar_url,"createdAt":self.created_at,"online":online})
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct Room {
    pub id: String,
    pub name: String,
    pub mode: Mode,
    pub deck_size: DeckSize,
    pub bet: i64,
    pub host: String,
    pub state: DurakState,
    pub seq: u64,
    pub events: Vec<Value>,
    pub pot: i64,
    pub payouts: HashMap<String, i64>,
    pub message: String,
}

async fn auth(state: &AppState, body: &Value) -> Result<String, Box<axum::response::Response>> {
    let id = body.get("clientId").and_then(Value::as_str).unwrap_or("");
    let token = body.get("token").and_then(Value::as_str).unwrap_or("");
    if !is_client_id(id)
        || !is_token(token)
        || (id.starts_with("discord:") && state.oauth.is_none())
    {
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "Неверный токен сессии",
        )));
    }
    if let Some(repository) = &state.repository {
        match super::verified_identity(repository, id, &super::oauth::digest(token)).await {
            Ok(true) => {}
            Ok(false) | Err(()) => {
                return Err(Box::new(error(
                    StatusCode::FORBIDDEN,
                    "Неверный токен сессии",
                )))
            }
        }
    }
    if state.sessions.read().await.get(id).map(String::as_str) == Some(token) {
        Ok(id.to_owned())
    } else {
        Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "Неверный токен сессии",
        )))
    }
}
async fn broadcast_lobby(state: &AppState, rooms: &HashMap<String, Room>) {
    let accounts = state.accounts.read().await;
    let clients = state.clients.read().await;
    let connected: HashSet<String> = clients.keys().cloned().collect();
    let history = if let Some(repository) = &state.repository {
        repository
            .lock()
            .await
            .list_history(8)
            .unwrap_or_default()
            .into_iter()
            .map(|h| json!({"room":h.room,"mode":h.mode,"at":h.at,"loser":h.loser,"bet":h.bet}))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let lobby = json!({"type":"lobby","online":clients.len(),"rooms":rooms.values().filter(|r| r.state.status != durak_engine::Status::Finished && r.state.players.iter().any(|p| !p.left && connected.contains(&p.id))).map(|r| room_summary(r, &accounts, &connected)).collect::<Vec<_>>(),"history":history});
    for client in clients.values() {
        client.send(axum::response::sse::Event::default().data(lobby.to_string()));
    }
}
async fn push_account(state: &AppState, id: &str) {
    let accounts = state.accounts.read().await;
    let clients = state.clients.read().await;
    if let Some(c) = clients.get(id) {
        if let Some(a) = accounts.get(id) {
            c.send(
                axum::response::sse::Event::default()
                    .data(json!({"type":"account","account":a.public(true)}).to_string()),
            );
        }
    }
}
fn settle(accounts: &mut HashMap<String, Account>, r: &mut Room, loser: Option<String>) {
    let winners: Vec<_> = r
        .state
        .players
        .iter()
        .filter(|p| Some(&p.id) != loser.as_ref())
        .map(|p| p.id.clone())
        .collect();
    r.payouts.clear();
    for p in &r.state.players {
        if let Some(a) = accounts.get_mut(&p.id) {
            a.games += 1;
            if loser.as_ref() == Some(&p.id) {
                a.losses += 1;
            } else if loser.is_some() {
                a.wins += 1;
            }
        }
    }
    if !winners.is_empty() {
        let share = r.pot / winners.len() as i64;
        let rem = r.pot % winners.len() as i64;
        for (i, id) in winners.iter().enumerate() {
            let amount = share + i64::from((i as i64) < rem);
            if let Some(a) = accounts.get_mut(id) {
                a.balance += amount;
            }
            r.payouts.insert(id.clone(), amount - r.bet);
        }
    }
    if let Some(id) = loser {
        r.payouts.insert(id, -r.bet);
    }
    r.pot = 0;
}

pub async fn watch(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<super::WatchQuery>,
) -> axum::response::Response {
    let id = q.id.unwrap_or_default();
    let token = q.token.unwrap_or_default();
    if !is_client_id(&id) || !is_token(&token) {
        return error(StatusCode::BAD_REQUEST, "Нужны id и токен сессии");
    };
    if let Some(repository) = &state.repository {
        match super::verified_identity(repository, &id, &super::oauth::digest(&token)).await {
            Ok(true) => {}
            Ok(false) | Err(()) => return error(StatusCode::FORBIDDEN, "Неверный токен сессии"),
        }
    }
    if state.sessions.read().await.get(&id) != Some(&token) {
        return error(StatusCode::FORBIDDEN, "Неверный токен сессии");
    };
    let until = q.until.unwrap_or_else(|| "any".into());
    if !["any", "game", "your-turn", "opponent", "finished"].contains(&until.as_str()) {
        return error(StatusCode::BAD_REQUEST, "Некорректный until");
    };
    let since = q.since.unwrap_or(0);
    let timeout = q.timeout.unwrap_or(20000).clamp(200, 60000);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
    let mut observed = since;
    loop {
        let rooms = state.rooms.read().await;
        let Some(r) = rooms
            .values()
            .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
        else {
            return Json(json!({"hit":false,"until":until,"since":since,"seq":0,"roomId":null,"events":[],"state":null})).into_response();
        };
        let accounts = state.accounts.read().await;
        let clients = state.clients.read().await;
        let connected = clients.keys().cloned().collect();
        let snap = snapshot(r, &id, &accounts, &connected);
        let matches = match until.as_str() {
            "your-turn" => [
                "canAttack",
                "canDefend",
                "canTransfer",
                "canTake",
                "canPass",
                "canStart",
            ]
            .iter()
            .any(|k| snap[*k] == true),
            "finished" => r.state.status == durak_engine::Status::Finished,
            "opponent" => r.events.iter().any(|e| {
                e["seq"].as_u64().unwrap_or(0) > since
                    && e["actorId"].as_str().is_some_and(|actor| actor != id)
            }),
            _ => true,
        };
        let hit = r.seq > since && matches;
        let expired = tokio::time::Instant::now() >= deadline;
        if hit || expired {
            return Json(json!({"hit":hit,"until":until,"since":since,"seq":r.seq,"roomId":r.id,"events":r.events.iter().rev().take(12).rev().collect::<Vec<_>>(),"state":snap})).into_response();
        };
        let rid = r.id.clone();
        observed = observed.max(r.seq);
        drop(clients);
        drop(accounts);
        drop(rooms);
        if tokio::time::timeout_at(deadline, state.event_hub.wait_for_change(&rid, observed))
            .await
            .is_err()
        {
            continue;
        }
    }
}

pub fn start_cleanup(state: super::AppState) {
    if cfg!(test) {
        return;
    }
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        loop {
            ticker.tick().await;
            cleanup_rooms(&state).await;
        }
    });
}

async fn cleanup_rooms(state: &super::AppState) {
    let now = now();
    let connected: HashSet<String> = state.clients.read().await.keys().cloned().collect();
    let mut rooms = state.rooms.write().await;
    let mut offline = state.offline_rooms.write().await;
    let mut removed = Vec::new();
    let mut refunds = Vec::new();
    for (id, room) in rooms.iter() {
        let has_connected = room
            .state
            .players
            .iter()
            .any(|p| !p.left && connected.contains(&p.id));
        if has_connected {
            offline.remove(id);
            continue;
        }
        let since = *offline.entry(id.clone()).or_insert(now);
        let stale = match room.state.status {
            durak_engine::Status::Playing | durak_engine::Status::Waiting => {
                now.saturating_sub(since) >= OFFLINE_GRACE_MS
            }
            durak_engine::Status::Finished => now.saturating_sub(since) >= FINISHED_ROOM_TTL_MS,
        };
        if stale {
            if room.state.status == durak_engine::Status::Playing && room.pot > 0 {
                refunds.push((
                    room.pot,
                    room.state
                        .players
                        .iter()
                        .map(|p| p.id.clone())
                        .collect::<Vec<_>>(),
                ));
            }
            removed.push(id.clone());
        }
    }
    for id in &removed {
        rooms.remove(id);
        offline.remove(id);
    }
    drop(offline);
    drop(rooms);
    if !refunds.is_empty() {
        let mut accounts = state.accounts.write().await;
        for (pot, players) in refunds {
            if players.is_empty() {
                continue;
            }
            let share = pot / players.len() as i64;
            let remainder = pot % players.len() as i64;
            for (index, id) in players.into_iter().enumerate() {
                if let Some(account) = accounts.get_mut(&id) {
                    account.balance += share + i64::from((index as i64) < remainder);
                    if let Some(repository) = &state.repository {
                        let _ = repository.lock().await.save_account(account);
                    }
                }
            }
        }
    }
    if let Some(repository) = &state.repository {
        let mut repository = repository.lock().await;
        for id in removed {
            let _ = repository.delete_transition(&id);
        }
    }
    if !connected.is_empty() {
        let rooms = state.rooms.read().await.clone();
        broadcast_lobby(state, &rooms).await;
    }
}

pub async fn connected(state: &super::AppState, id: &str) {
    let mut rooms = state.rooms.write().await;
    if let Some(r) = rooms
        .values_mut()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        broadcast(state, r, id, "connect", &json!({})).await;
    } else if let Some(c) = state.clients.read().await.get(id) {
        c.send(axum::response::sse::Event::default().data(json!({"type":"noroom"}).to_string()));
    }
    broadcast_lobby(state, &rooms).await;
}
pub async fn disconnected(state: &AppState, id: &str) {
    let mut rooms = state.rooms.write().await;
    if let Some(r) = rooms
        .values_mut()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        broadcast(state, r, id, "disconnect", &json!({})).await;
    }
    broadcast_lobby(state, &rooms).await;
}
pub async fn api_status(state: &AppState) -> impl IntoResponse {
    let rooms = state.rooms.read().await;
    Json(
        json!({"online":state.clients.read().await.len(),"clients":[],"rooms":rooms.values().map(|r|json!({"id":r.id,"status":status(r.state.status),"bet":r.bet,"pot":r.pot,"players":r.state.players.len()})).collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
mod tests;
