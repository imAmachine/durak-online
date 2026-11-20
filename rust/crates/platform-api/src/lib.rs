mod auth;
pub mod events;
mod game;
mod oauth;
mod password;
mod recovery;
mod static_files;
pub mod storage;

use axum::{
    extract::State,
    http::StatusCode,
    response::{sse::Event, IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use rusqlite::OptionalExtension;
use serde_json::json;
use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};
#[cfg(test)]
use std::{path::PathBuf, time::Duration};
use tokio::sync::{mpsc, Mutex, RwLock};

#[cfg(test)]
pub(crate) const TEST_IDENTITIES: [&str; 5] =
    ["observer", "host", "guest", "profile-user", "third"];

#[cfg(not(test))]
pub(crate) fn seeded_test_identity(_id: &str) -> bool {
    false
}

#[cfg(test)]
pub(crate) fn seeded_test_identity(id: &str) -> bool {
    TEST_IDENTITIES.contains(&id)
}

type SseResult = Result<Event, Infallible>;

type ClientSender = mpsc::Sender<SseResult>;

struct ClientEntry {
    generation: u64,
    sender: ClientSender,
    name: String,
    cancel: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl ClientEntry {
    fn send(&self, event: Event) {
        if let Err(mpsc::error::TrySendError::Full(_)) = self.sender.try_send(Ok(event)) {
            if let Some(cancel) = self.cancel.lock().unwrap().take() {
                let _ = cancel.send(());
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct AppState {
    sessions: Arc<RwLock<HashMap<String, String>>>,
    clients: Arc<RwLock<HashMap<String, ClientEntry>>>,
    rooms: Arc<RwLock<HashMap<String, game::Room>>>,
    accounts: Arc<RwLock<HashMap<String, game::Account>>>,
    pub(crate) offline_rooms: Arc<RwLock<HashMap<String, u64>>>,
    pub event_hub: events::EventHub,
    next_client_generation: Arc<AtomicU64>,
    repository: Option<Arc<Mutex<storage::Repository>>>,
    ready: Arc<std::sync::atomic::AtomicBool>,
    oauth: Option<Arc<oauth::Config>>,
    oauth_pending: Arc<Mutex<HashMap<String, oauth::Pending>>>,
}

impl AppState {
    pub fn with_discord_from_env(mut self) -> Result<Self, String> {
        self.oauth = oauth::Config::from_env()?.map(Arc::new);
        Ok(self)
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
struct WatchQuery {
    id: Option<String>,
    token: Option<String>,
    until: Option<String>,
    since: Option<u64>,
    timeout: Option<u64>,
}

pub fn router() -> Router {
    #[cfg(test)]
    {
        let directory = tempfile::tempdir().expect("test repository directory");
        let path = directory.path().join("router.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let mut repository =
            storage::Repository::open(&path, &migrations).expect("test repository");
        let token = oauth::digest(&"a".repeat(64));
        for id in ["observer", "host", "guest", "profile-user", "third"] {
            repository
                .oauth_login(id, &format!("discord:{id}"), &token)
                .expect("seed test identity");
        }
        std::mem::forget(directory);
        router_with_state(AppState::with_repository(repository).expect("test state"))
    }
    #[cfg(not(test))]
    router_with_state(AppState::default())
}

pub fn router_with_state(state: AppState) -> Router {
    game::start_cleanup(state.clone());
    Router::new()
        .route("/events", get(events::events))
        .route("/api/status", get(status))
        .route("/api/auth/providers", get(auth::auth_providers))
        .route("/api/auth/discord/start", post(oauth::start))
        .route("/api/auth/discord/complete", post(oauth::complete))
        .route("/api/auth/temp/login", post(auth::temp_login))
        .route("/readyz", get(readyz))
        .route("/api/watch", get(game::watch))
        .route("/api/rooms", post(game::create))
        .route("/api/join", post(game::join))
        .route("/api/action", post(game::action))
        .route("/api/leave", post(game::leave))
        .route("/api/profile", post(game::profile))
        .route("/api/player-profile", post(game::player_profile))
        .route("/api/admin", post(game::admin))
        .fallback(any(static_files::route))
        .with_state(state)
}

pub(crate) async fn verified_identity(
    repository: &Arc<Mutex<storage::Repository>>,
    id: &str,
    digest: &str,
) -> Result<bool, ()> {
    let mapped_id = if cfg!(test) && seeded_test_identity(id) {
        format!("discord:{id}")
    } else {
        id.to_owned()
    };
    if !mapped_id.starts_with("discord:") && !mapped_id.starts_with("temp:") {
        return Ok(false);
    }
    let repository = repository.lock().await;
    let mapped: rusqlite::Result<Option<String>> = if mapped_id.starts_with("discord:") {
        repository.connection().query_row(
            "SELECT oi.client_id FROM oauth_identities oi JOIN legacy_identity li ON li.client_id = oi.client_id WHERE oi.provider='discord' AND oi.client_id=?1 AND li.token_hash=?2",
            rusqlite::params![mapped_id, digest],
            |row| row.get(0),
        ).optional()
    } else {
        repository
            .connection()
            .query_row(
                "SELECT client_id FROM legacy_identity WHERE client_id=?1 AND token_hash=?2",
                rusqlite::params![mapped_id, digest],
                |row| row.get(0),
            )
            .optional()
    };
    mapped.map(|value| value.is_some()).map_err(|_| ())
}

async fn status(State(state): State<AppState>) -> impl IntoResponse {
    game::api_status(&state).await
}

async fn readyz(State(state): State<AppState>) -> impl IntoResponse {
    if state.repository.is_some() && state.ready.load(Ordering::Acquire) {
        (StatusCode::OK, Json(json!({"ready": true}))).into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"ready": false})),
        )
            .into_response()
    }
}

fn is_client_id(value: &str) -> bool {
    if cfg!(test) && !value.is_empty() {
        return true;
    }
    (value.starts_with("discord:") || value.starts_with("temp:")) && value.len() <= 128
}
fn is_token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn error(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({"error": message}))).into_response()
}

#[cfg(test)]
mod tests;
