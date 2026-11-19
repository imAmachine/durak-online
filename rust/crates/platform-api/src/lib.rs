pub mod events;
mod game;
mod oauth;
mod password;
pub mod storage;

use rusqlite::OptionalExtension;
use axum::{
    extract::{Query, State},
    http::{header, StatusCode, Uri},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{any, get, post},
    Json, Router,
};
use futures_core::Stream;
use serde_json::json;
use std::{
    collections::HashMap,
    convert::Infallible,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_stream::wrappers::ReceiverStream;

#[cfg(test)]
pub(crate) const TEST_IDENTITIES: [&str; 5] = ["observer", "host", "guest", "profile-user", "third"];

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
}

#[derive(Clone, Default)]
pub struct AppState {
    sessions: Arc<RwLock<HashMap<String, String>>>,
    clients: Arc<RwLock<HashMap<String, ClientEntry>>>,
    rooms: Arc<RwLock<HashMap<String, game::Room>>>,
    accounts: Arc<RwLock<HashMap<String, game::Account>>>,
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

    pub fn with_repository(repository: storage::Repository) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
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
        let mut accounts = repository.list_accounts()?.into_iter().map(|account| (account.id.clone(), account)).collect::<HashMap<_, _>>();
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
                    } else if let Some(stored) = player.id.strip_prefix("temp:").filter(|rest| !rest.is_empty()) {
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
            let invalid = || std::io::Error::new(std::io::ErrorKind::InvalidData, format!("invalid room snapshot {}", saved.game_id));
            if room.id != saved.game_id || room.seq.checked_sub(1).and_then(|n| i64::try_from(n).ok()) != Some(saved.seq)
                || room.events.last().and_then(|e| e.get("seq")).and_then(|s| s.as_u64()) != Some(room.seq)
                || room.state.players.len() > 6 || room.bet < 0 || room.pot < 0
                || room.mode != room.state.mode || room.deck_size != room.state.deck_size {
                return Err(invalid().into());
            }
            let mut players = std::collections::HashSet::new();
            for player in &room.state.players {
                if !players.insert(player.id.clone()) || (!player.left && !seated.insert(player.id.clone())) {
                    return Err(invalid().into());
                }
                if !accounts.contains_key(&player.id) {
                    let account = repository.load_account(&player.id)?.ok_or_else(invalid)?;
                    accounts.insert(player.id.clone(), account);
                }
            }
            if !room.state.players.is_empty() {
                if !players.contains(&room.host) { return Err(invalid().into()); }
                rooms.insert(room.id.clone(), room);
            }
        }
        Ok(Self {
            rooms: Arc::new(RwLock::new(rooms)),
            accounts: Arc::new(RwLock::new(accounts)),
            repository: Some(Arc::new(Mutex::new(repository))),
            ready: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            ..Self::default()
        })
    }
}

#[derive(Debug, serde::Deserialize)]
struct EventsQuery {
    id: Option<String>,
    token: Option<String>,
    name: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct WatchQuery {
    id: Option<String>,
    token: Option<String>,
    until: Option<String>,
    since: Option<u64>,
    timeout: Option<u64>,
}

struct ClientStream {
    inner: ReceiverStream<SseResult>,
    state: AppState,
    id: String,
    generation: u64,
}

impl Stream for ClientStream {
    type Item = SseResult;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}

impl Drop for ClientStream {
    fn drop(&mut self) {
        let state = self.state.clone();
        let id = self.id.clone();
        let generation = self.generation;
        tokio::spawn(async move {
            let mut clients = state.clients.write().await;
            if clients.get(&id).map(|entry| entry.generation) == Some(generation) {
                clients.remove(&id);
                drop(clients);
                game::disconnected(&state, &id).await;
            }
        });
    }
}

pub fn router() -> Router {
    #[cfg(test)]
    {
        let directory = tempfile::tempdir().expect("test repository directory");
        let path = directory.path().join("router.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let mut repository = storage::Repository::open(&path, &migrations).expect("test repository");
        let token = oauth::digest(&"a".repeat(64));
        for id in ["observer", "host", "guest", "profile-user", "third"] {
            repository.oauth_login(id, &format!("discord:{id}"), &token).expect("seed test identity");
        }
        std::mem::forget(directory);
        return router_with_state(AppState::with_repository(repository).expect("test state"));
    }
    #[cfg(not(test))]
    router_with_state(AppState::default())
}

pub fn router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/events", get(events))
        .route("/api/status", get(status))
        .route("/api/auth/providers", get(auth_providers))
        .route("/api/auth/discord/start", post(oauth::start))
        .route("/api/auth/discord/complete", post(oauth::complete))
        .route("/api/auth/temp/login", post(temp_login))
        .route("/readyz", get(readyz))
        .route("/api/watch", get(game::watch))
        .route("/api/rooms", post(game::create))
        .route("/api/join", post(game::join))
        .route("/api/action", post(game::action))
        .route("/api/leave", post(game::leave))
        .route("/api/profile", post(game::profile))
        .route("/api/admin", post(game::admin))
        .fallback(any(route))
        .with_state(state)
}

async fn auth_providers() -> Json<serde_json::Value> {
    Json(json!({"providers": ["temporary"]}))
}

async fn temp_login(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name = body.get("name").and_then(|value| value.as_str()).unwrap_or("").trim();
    let password = body.get("password").and_then(|value| value.as_str()).unwrap_or("");
    if name.is_empty() || name.chars().count() > 20 || password.len() < 4 || password.len() > 128 {
        return error(StatusCode::BAD_REQUEST, "Укажите имя и пароль от 4 до 128 символов");
    }
    let repository = match &state.repository {
        Some(repository) => repository,
        None => return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно"),
    };
    let matches = |account: &game::Account| {
        account.password_hash.as_deref().is_some_and(|stored| password::verify(password, stored))
    };
    let mut account = {
        let accounts = state.accounts.read().await;
        accounts
            .values()
            .find(|account| account.name.to_lowercase() == name.to_lowercase() && matches(account))
            .cloned()
            .map(|account| (account, false))
    };
    if account.is_none() {
        let stored = repository.lock().await.list_accounts().unwrap_or_default();
        account = stored
            .into_iter()
            .find(|account| account.name.to_lowercase() == name.to_lowercase() && matches(account))
            .map(|account| {
                let upgrade = password::needs_upgrade(account.password_hash.as_deref().unwrap_or_default());
                (account, upgrade)
            });
    }
    let Some((stored, upgrade)) = account else {
        password::burn(password);
        return error(StatusCode::FORBIDDEN, "Неверное имя или пароль");
    };
    let password_hash = if upgrade { password::hash(password) } else { stored.password_hash.clone().unwrap_or_default() };
    let account = game::Account { password_hash: Some(password_hash.clone()), ..stored };
    let token = match oauth::random() { Ok(token) => oauth::digest(&token), Err(response) => return response };
    {
        let mut repository = repository.lock().await;
        if repository.finish_temp_login(&account.id, &oauth::digest(&token), &password_hash).is_err() {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
        if repository.save_account(&account).is_err() {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
    }
    state.accounts.write().await.insert(account.id.clone(), account.clone());
    state.sessions.write().await.insert(account.id.clone(), token.clone());
    state.clients.write().await.remove(&account.id);
    Json(json!({"clientId": account.id, "token": token})).into_response()
}

pub(crate) async fn verified_identity(repository: &Arc<Mutex<storage::Repository>>, id: &str, digest: &str) -> Result<bool, ()> {
    let mapped_id = if cfg!(test) && seeded_test_identity(id) { format!("discord:{id}") } else { id.to_owned() };
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
        repository.connection().query_row(
            "SELECT client_id FROM legacy_identity WHERE client_id=?1 AND token_hash=?2",
            rusqlite::params![mapped_id, digest],
            |row| row.get(0),
        ).optional()
    };
    mapped.map(|value| value.is_some()).map_err(|_| ())
}

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, Response> {
    let id = query.id.as_deref().filter(|value| is_client_id(value)).map(str::to_owned);
    let token = query.token.as_deref().filter(|value| is_token(value)).map(str::to_owned);
    let (Some(id), Some(token)) = (id, token) else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Нужны корректные id и токен сессии",
        ));
    };

    {
        let mut sessions = state.sessions.write().await;
        let Some(repository) = &state.repository else {
            return Err(error(StatusCode::FORBIDDEN, "Неверный токен сессии"));
        };
        let verified_id = if cfg!(test) && seeded_test_identity(&id) { format!("discord:{id}") } else { id.clone() };
        if verified_id.starts_with("discord:") && state.oauth.is_none() && !(cfg!(test) && seeded_test_identity(&id)) {
            return Err(error(StatusCode::FORBIDDEN, "Неверный токен сессии"));
        }
        match verified_identity(repository, &verified_id, &oauth::digest(&token)).await {
            Ok(true) => {}
            Ok(false) => return Err(error(StatusCode::FORBIDDEN, "Неверный токен сессии")),
            Err(()) => return Err(error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно")),
        }
        if let Some(previous) = sessions.get(&id) {
            if previous != &token {
                return Err(error(StatusCode::FORBIDDEN, "Неверный токен сессии"));
            }
        } else {
            sessions.insert(id.clone(), token);
        }
    }

    let name: String = query
        .name
        .unwrap_or_else(|| "Игрок".into())
        .chars()
        .take(20)
        .collect();
    let account = {
        let mut accounts = state.accounts.write().await;
        if !accounts.contains_key(&id) {
            let stored = if let Some(repository) = &state.repository {
                match repository.lock().await.load_account(&id) {
                    Ok(value) => value,
                    Err(_) => return Err(error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно")),
                }
            } else { None };
            accounts.insert(id.clone(), stored.unwrap_or_else(|| game::Account::new(&id, &name)));
        }
        let account = accounts.get_mut(&id).unwrap();
        if let Some(repository) = &state.repository {
            if repository.lock().await.save_account(account).is_err() {
                return Err(error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно"));
            }
        }
        account.public(true)
    };
    let retry = Event::default().retry(Duration::from_secs(2));
    let (sender, receiver) = mpsc::channel(128);
    let generation = state.next_client_generation.fetch_add(1, Ordering::Relaxed);
    {
        let mut clients = state.clients.write().await;
        clients.insert(
            id.clone(),
            ClientEntry {
                generation,
                sender: sender.clone(),
                name,
            },
        );
    }
    for event in [
        retry,
        Event::default().data(json!({"type":"account", "account": account}).to_string()),
    ] {
        let _ = sender.send(Ok(event)).await;
    }
    game::connected(&state, &id).await;
    let stream = ClientStream {
        inner: ReceiverStream::new(receiver),
        state,
        id,
        generation,
    };
    Ok(Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
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

async fn route(uri: Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") {
        return error(
            StatusCode::NOT_IMPLEMENTED,
            "Этот legacy endpoint ещё не реализован в Rust API",
        );
    }
    let (file, mime) = match path {
        "/" | "/index.html" => ("index.html", "text/html"),
        "/admin" => ("admin.html", "text/html"),
        "/admin.html" => ("admin.html", "text/html"),
        "/app.js" => ("app.js", "application/javascript"),
        "/style.css" => ("style.css", "text/css"),
        _ => return error(StatusCode::NOT_FOUND, "Not found"),
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    match tokio::fs::read(root.join(file)).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, mime)], bytes).into_response(),
        Err(_) => error(StatusCode::NOT_FOUND, "Not found"),
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
mod tests {
    use super::*;
    use axum::{body::Body, http::Request, response::Response};
    use tempfile::tempdir;
    use tower::ServiceExt;

    struct TestStream { stream: axum::body::BodyDataStream, buffer: String }

    async fn open_events(app: &Router, id: &str) -> TestStream {
        let token = "a".repeat(64);
        let response = app.clone().oneshot(Request::get(format!("/events?id={id}&token={token}&name={id}"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        TestStream { stream: response.into_body().into_data_stream(), buffer: String::new() }
    }

    async fn lobby(stream: &mut TestStream, predicate: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
        use tokio_stream::StreamExt;
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                while let Some(end) = stream.buffer.find("\n\n") {
                    let frame = stream.buffer[..end].to_owned();
                    stream.buffer.drain(..end + 2);
                    if let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data: ")) {
                        let value: serde_json::Value = serde_json::from_str(data).unwrap();
                        if value["type"] == "lobby" && predicate(&value) { return value; }
                    }
                }
                let bytes = stream.stream.next().await.unwrap().unwrap();
                stream.buffer.push_str(std::str::from_utf8(&bytes).unwrap());
            }
        }).await.expect("lobby update timed out")
    }

    async fn post(app: &Router, path: &str, value: serde_json::Value) -> serde_json::Value {
        let response = app.clone().oneshot(Request::post(path).header("content-type", "application/json")
            .body(Body::from(value.to_string())).unwrap()).await.unwrap();
        assert!(response.status().is_success(), "{}", response.status());
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn admin_creates_rotates_and_deletes_temporary_accounts() {
        std::env::set_var("ADMIN_KEY", "panel-secret");
        let dir = tempdir().unwrap();
        let path = dir.path().join("admin.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let state = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        let app = router_with_state(state.clone());
        let created = post(&app, "/api/admin", json!({"key":"panel-secret","op":"create_temp","name":"Гость","password":"первый-пароль"})).await;
        let id = created["account"]["id"].as_str().unwrap().to_owned();
        let stored = state.repository.as_ref().unwrap().lock().await.load_account(&id).unwrap().unwrap();
        let hash = stored.password_hash.clone().unwrap();
        assert!(hash.starts_with("pbkdf2_sha256$") && !hash.contains("первый-пароль"));
        let login = post(&app, "/api/auth/temp/login", json!({"name":"гость","password":"первый-пароль"})).await;
        assert_eq!(login["clientId"], id);
        let token = login["token"].as_str().unwrap().to_owned();
        let stream = app.clone().oneshot(Request::get(format!("/events?id={id}&token={token}&name=Гость"))
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(stream.status(), StatusCode::OK);
        let changed = post(&app, "/api/admin", json!({"key":"panel-secret","op":"update","id":id,"password":"второй-пароль"})).await;
        assert_eq!(changed["sessionRevoked"], true);
        assert!(!state.sessions.read().await.contains_key(&id));
        assert!(state.clients.read().await.contains_key(&id));
        let stale = app.clone().oneshot(Request::post("/api/auth/temp/login").header("content-type","application/json")
            .body(Body::from(json!({"name":"Гость","password":"первый-пароль"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(stale.status(), StatusCode::FORBIDDEN);
        let list = post(&app, "/api/admin", json!({"key":"panel-secret","op":"list"})).await;
        let row = list["players"].as_array().unwrap().iter().find(|p| p["id"] == id.as_str()).cloned().unwrap();
        assert_eq!(row["kind"], "temp");
        assert_eq!(row["hasPassword"], true);
        let deleted = post(&app, "/api/admin", json!({"key":"panel-secret","op":"delete","id":id})).await;
        assert_eq!(deleted["deleted"], id.as_str());
        assert!(!state.accounts.read().await.contains_key(&id));
        assert!(state.repository.as_ref().unwrap().lock().await.load_account(&id).unwrap().is_none());
        assert!(state.sessions.read().await.is_empty());
        let rejected = app.clone().oneshot(Request::post("/api/admin").header("content-type","application/json")
            .body(Body::from(json!({"key":"panel-secret","op":"delete","id":id}).to_string())).unwrap()).await.unwrap();
        assert_eq!(rejected.status(), StatusCode::NOT_FOUND);
        let wrong = app.clone().oneshot(Request::post("/api/auth/temp/login").header("content-type","application/json")
            .body(Body::from(json!({"name":"Гость","password":"второй-пароль"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
        std::env::remove_var("ADMIN_KEY");
    }

    #[tokio::test]
    async fn lobby_stream_tracks_room_lifecycle_and_connections() {
        let app = router();
        let mut observer = open_events(&app, "observer").await;
        let mut host = open_events(&app, "host").await;
        let guest = open_events(&app, "guest").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"name":"Test"})).await;
        let code = created["code"].as_str().unwrap();
        let room = |v: &serde_json::Value| v["rooms"].as_array().unwrap().iter().find(|r| r["id"] == code).cloned();
        lobby(&mut observer, |v| room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 1)).await;
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code})).await;
        lobby(&mut observer, |v| room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 2)).await;
        drop(guest);
        lobby(&mut observer, |v| v["online"] == 2 && room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 1)).await;
        let _guest = open_events(&app, "guest").await;
        lobby(&mut observer, |v| room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 2)).await;
        post(&app, "/api/action", json!({"clientId":"host","token":token,"action":"start"})).await;
        lobby(&mut observer, |v| room(v).is_some_and(|r| r["status"] == "playing" && r["players"][0]["cards"] == 6)).await;
        post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
        lobby(&mut observer, |v| room(v).is_none()).await;
        lobby(&mut host, |v| room(v).is_none()).await;
    }

    #[tokio::test]
    async fn lobby_removes_empty_room_after_leave() {
        let app = router();
        let mut observer = open_events(&app, "observer").await;
        let _host = open_events(&app, "host").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token})).await;
        let code = created["code"].as_str().unwrap();
        lobby(&mut observer, |v| v["rooms"].as_array().unwrap().iter().any(|r| r["id"] == code)).await;
        post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
        lobby(&mut observer, |v| v["rooms"].as_array().unwrap().is_empty()).await;
    }

    #[tokio::test]
    async fn profile_persists_after_reopening_repository() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("profile.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let app = router_with_state(AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap());
        let stream = open_events(&app, "profile-user").await;
        post(&app, "/api/profile", json!({"clientId":"profile-user", "token":"a".repeat(64), "name":"Saved"})).await;
        drop(stream);
        drop(app);
        let repo = storage::Repository::open(&path, &migrations).unwrap();
        assert_eq!(repo.load_account("profile-user").unwrap().unwrap().name, "Saved");
    }

    #[tokio::test]
    async fn failed_profile_write_does_not_change_memory() {
        let directory = tempdir().unwrap();
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let repository = storage::Repository::open(directory.path().join("failure.sqlite"), migrations).unwrap();
        let state = AppState::with_repository(repository).unwrap();
        let app = router_with_state(state.clone());
        let _stream = open_events(&app, "profile-user").await;
        state.repository.as_ref().unwrap().lock().await.connection().execute_batch("CREATE TRIGGER reject_profile BEFORE UPDATE ON legacy_accounts BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
        let response = app.clone().oneshot(Request::post("/api/profile").header("content-type", "application/json").body(Body::from(json!({"clientId":"profile-user", "token":"a".repeat(64), "name":"Rejected"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_ne!(state.accounts.read().await["profile-user"].name, "Rejected");
    }

    #[tokio::test]
    async fn failed_game_transition_keeps_room_accounts_and_events_unchanged() {
        let directory = tempdir().unwrap();
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let repository = storage::Repository::open(directory.path().join("game.sqlite"), migrations).unwrap();
        let state = AppState::with_repository(repository).unwrap();
        let app = router_with_state(state.clone());
        let _host = open_events(&app, "host").await;
        let _guest = open_events(&app, "guest").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"bet":50})).await;
        let code = created["code"].as_str().unwrap();
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code})).await;
        let before = serde_json::to_value(&state.rooms.read().await[code]).unwrap();
        state.repository.as_ref().unwrap().lock().await.connection().execute_batch("CREATE TRIGGER reject_game BEFORE INSERT ON durable_game_transitions BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
        let response = app.clone().oneshot(Request::post("/api/action").header("content-type", "application/json").body(Body::from(json!({"clientId":"host","token":token,"action":"start"}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(serde_json::to_value(&state.rooms.read().await[code]).unwrap(), before);
        assert_eq!(state.accounts.read().await["host"].balance, 1000);
        assert_eq!(state.accounts.read().await["guest"].balance, 1000);
        let repo = state.repository.as_ref().unwrap().lock().await;
        assert_eq!(repo.load_account("host").unwrap().unwrap().balance, 1000);
        assert_eq!(repo.load_account("guest").unwrap().unwrap().balance, 1000);
        let (_, stored) = repo.load_transition_state(code).unwrap().unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&stored).unwrap(), before);
    }

    #[tokio::test]
    async fn restarted_game_restores_offline_players_and_settles_once() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("restart.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let state = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        let app = router_with_state(state.clone());
        let host = open_events(&app, "host").await;
        let guest = open_events(&app, "guest").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"bet":50})).await;
        let code = created["code"].as_str().unwrap().to_string();
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code})).await;
        post(&app, "/api/action", json!({"clientId":"host","token":token,"action":"start"})).await;
        drop(host);
        drop(guest);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if state.clients.read().await.is_empty() { break; }
                tokio::task::yield_now().await;
            }
            let _guard = state.rooms.write().await;
        }).await.unwrap();
        let before = serde_json::to_value(&state.rooms.read().await[&code]).unwrap();
        drop(app);
        drop(state);
        let restored = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(&restored.rooms.read().await[&code]).unwrap(), before);
        assert!(restored.clients.read().await.is_empty());
        assert_eq!(restored.accounts.read().await["guest"].balance, 950);
        let restarted_app = router_with_state(restored.clone());
        let _restored_host = open_events(&restarted_app, "host").await;
        post(&restarted_app, "/api/leave", json!({"clientId":"host","token":token})).await;
        assert_eq!(restored.accounts.read().await["host"].balance, 950);
        assert_eq!(restored.accounts.read().await["guest"].balance, 1050);
        assert_eq!(restored.accounts.read().await["guest"].wins, 1);
        let again = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        assert_eq!(again.accounts.read().await["guest"].balance, 1050);
        assert_eq!(again.accounts.read().await["guest"].games, 1);
        assert_eq!(again.rooms.read().await[&code].state.status, durak_engine::Status::Finished);
    }

    #[tokio::test]
    async fn new_accounts_create_join_and_reopen_with_matching_sequences() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("new.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let state = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        let app = router_with_state(state.clone());
        let host = open_events(&app, "host").await;
        let guest = open_events(&app, "guest").await;
        state.accounts.write().await.remove("host");
        state.accounts.write().await.remove("guest");
        state.repository.as_ref().unwrap().lock().await.connection().execute("DELETE FROM legacy_accounts WHERE client_id IN ('host','guest')", []).unwrap();
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"name":"Saved table","playerName":"Alice"})).await;
        let code = created["code"].as_str().unwrap().to_owned();
        assert_eq!(created["room"]["id"], code);
        assert_eq!(created["room"]["name"], "Saved table");
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code,"playerName":"Bob"})).await;
        let room = state.rooms.read().await[&code].clone();
        let repo = state.repository.as_ref().unwrap().lock().await;
        let saved = repo.load_transition_snapshot(&code).unwrap().unwrap();
        assert_eq!(saved.seq as u64 + 1, room.seq);
        let payload: serde_json::Value = serde_json::from_str(&saved.state_json).unwrap();
        assert_eq!(payload["seq"], room.seq);
        assert_eq!(payload["events"].as_array().unwrap().last().unwrap()["seq"], room.seq);
        assert_eq!(repo.load_account("host").unwrap().unwrap().name, "Alice");
        assert_eq!(repo.load_account("guest").unwrap().unwrap().name, "Bob");
        drop(repo);
        drop(host);
        drop(guest);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop { if state.clients.read().await.is_empty() { break; } tokio::task::yield_now().await; }
            let _guard = state.rooms.write().await;
        }).await.unwrap();
        drop(app);
        drop(state);
        let restored = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        assert_eq!(restored.rooms.read().await[&code].name, "Saved table");
        assert_eq!(restored.accounts.read().await["host"].name, "Alice");
        assert_eq!(restored.accounts.read().await["guest"].name, "Bob");
    }

    #[tokio::test]
    async fn stale_player_name_from_client_does_not_rename_account() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("names.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let state = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        let app = router_with_state(state.clone());
        let host = open_events(&app, "host").await;
        let guest = open_events(&app, "guest").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"name":"Стол","playerName":"Игрок"})).await;
        let code = created["code"].as_str().unwrap().to_owned();
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code,"playerName":"Игрок"})).await;
        assert_eq!(state.accounts.read().await["host"].name, "host");
        assert_eq!(state.accounts.read().await["guest"].name, "guest");
        assert_eq!(state.repository.as_ref().unwrap().lock().await.load_account("guest").unwrap().unwrap().name, "guest");
        drop(host);
        drop(guest);
    }

    #[tokio::test]
    async fn player_can_rejoin_finished_room_and_recover_after_restart() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("rejoin.sqlite");
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let state = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        let app = router_with_state(state.clone());
        let host = open_events(&app, "host").await;
        let guest = open_events(&app, "guest").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"bet":50})).await;
        let code = created["code"].as_str().unwrap().to_string();
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code})).await;
        post(&app, "/api/action", json!({"clientId":"host","token":token,"action":"start"})).await;
        post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
        let result = post(&app, "/api/join", json!({"clientId":"host","token":token,"roomId":code})).await;
        assert_eq!(result["room"]["id"], code);
        {
            let rooms = state.rooms.read().await;
            let room = &rooms[&code];
            assert_eq!(room.state.players.iter().filter(|p| p.id == "host").count(), 1);
            assert!(!room.state.players.iter().find(|p| p.id == "host").unwrap().left);
            assert_eq!(room.state.status, durak_engine::Status::Finished);
        }
        drop(host);
        drop(guest);
        tokio::time::timeout(Duration::from_secs(2), async {
            loop { if state.clients.read().await.is_empty() { break; } tokio::task::yield_now().await; }
            let _guard = state.rooms.write().await;
        }).await.unwrap();
        drop(app);
        drop(state);
        let recovered = AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
        let rooms = recovered.rooms.read().await;
        let room = &rooms[&code];
        assert_eq!(room.state.players.iter().filter(|p| p.id == "host").count(), 1);
        assert!(!room.state.players.iter().find(|p| p.id == "host").unwrap().left);
        assert_eq!(recovered.accounts.read().await["host"].losses, 1);
        assert_eq!(recovered.accounts.read().await["guest"].wins, 1);
    }

    #[tokio::test]
    async fn finished_game_can_restart_without_repeating_settlement() {
        let dir = tempdir().unwrap();
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let state = AppState::with_repository(storage::Repository::open(dir.path().join("finished.sqlite"), migrations).unwrap()).unwrap();
        let app = router_with_state(state.clone());
        let _host = open_events(&app, "host").await;
        let _guest = open_events(&app, "guest").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"bet":50})).await;
        let code = created["code"].as_str().unwrap();
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code})).await;
        post(&app, "/api/action", json!({"clientId":"host","token":token,"action":"start"})).await;
        post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
        let _third = open_events(&app, "third").await;
        post(&app, "/api/join", json!({"clientId":"third","token":token,"roomId":code})).await;
        post(&app, "/api/action", json!({"clientId":"guest","token":token,"action":"start"})).await;
        let accounts = state.accounts.read().await;
        assert_eq!(accounts["host"].balance, 950);
        assert_eq!(accounts["guest"].balance, 1000);
        assert_eq!(accounts["guest"].games, 1);
        assert_eq!(accounts["guest"].wins, 1);
        assert_eq!(accounts["third"].balance, 950);
        drop(accounts);
        assert_eq!(state.rooms.read().await[code].pot, 100);
        assert_eq!(state.rooms.read().await[code].state.status, durak_engine::Status::Playing);
    }

    #[tokio::test]
    async fn rejected_leave_rolls_back_settlement_and_room() {
        let dir = tempdir().unwrap();
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let state = AppState::with_repository(storage::Repository::open(dir.path().join("leave.sqlite"), migrations).unwrap()).unwrap();
        let app = router_with_state(state.clone());
        let _host = open_events(&app, "host").await;
        let _guest = open_events(&app, "guest").await;
        let token = "a".repeat(64);
        let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token,"bet":50})).await;
        let code = created["code"].as_str().unwrap();
        post(&app, "/api/join", json!({"clientId":"guest","token":token,"roomId":code})).await;
        post(&app, "/api/action", json!({"clientId":"host","token":token,"action":"start"})).await;
        let before = serde_json::to_value(&state.rooms.read().await[code]).unwrap();
        state.repository.as_ref().unwrap().lock().await.connection().execute_batch("CREATE TRIGGER reject_settlement BEFORE UPDATE ON legacy_accounts BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
        let response = app.clone().oneshot(Request::post("/api/leave").header("content-type", "application/json").body(Body::from(json!({"clientId":"host","token":token}).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(serde_json::to_value(&state.rooms.read().await[code]).unwrap(), before);
        assert_eq!(state.accounts.read().await["host"].balance, 950);
        assert_eq!(state.accounts.read().await["guest"].balance, 950);
        assert_eq!(state.accounts.read().await["guest"].games, 0);
        let repo = state.repository.as_ref().unwrap().lock().await;
        assert_eq!(repo.load_account("guest").unwrap().unwrap().balance, 950);
        assert_eq!(serde_json::from_str::<serde_json::Value>(&repo.load_transition_state(code).unwrap().unwrap().1).unwrap(), before);
    }

    #[test]
    fn invalid_recovery_snapshot_prevents_startup() {
        let dir = tempdir().unwrap();
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let repository = storage::Repository::open(dir.path().join("invalid.sqlite"), migrations).unwrap();
        repository.connection().execute("INSERT INTO durable_game_states(game_id,seq,state_json) VALUES ('bad',0,'{}')", []).unwrap();
        assert!(AppState::with_repository(repository).is_err());
    }

    #[tokio::test]
    async fn readiness_requires_a_migrated_repository() {
        let directory = tempdir().unwrap();
        let repository = storage::Repository::open(
            directory.path().join("durak.sqlite"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations"),
        )
        .unwrap();
        let response: Response = router_with_state(AppState::with_repository(repository).unwrap())
            .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response: Response = router_with_state(AppState::default())
            .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn unlinked_active_room_fails_closed_at_startup_without_deleting_rows() {
        let dir = tempdir().unwrap();
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let mut repo = storage::Repository::open(dir.path().join("orphan.sqlite"), &migrations).unwrap();
        let room = game::Room { id: "ORPHAN".into(), name: "Locked stakes".into(), mode: durak_engine::Mode::Classic, deck_size: durak_engine::DeckSize::ThirtySix, bet: 50, host: "discord:legacy".into(), state: durak_engine::DurakState::new(vec!["discord:legacy".into(), "discord:ok".into()], durak_engine::Mode::Classic, durak_engine::DeckSize::ThirtySix), seq: 1, events: vec![json!({"seq":1})], pot: 100, payouts: HashMap::new(), message: "playing".into() };
        let transition = storage::DurableTransition { game_id: "ORPHAN".into(), expected_seq: None, seq: 0, state_json: serde_json::to_string(&room).unwrap(), accounts: vec![] };
        repo.commit_transition(&transition).unwrap();
        let result = AppState::with_repository(repo);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn invalid_events_credentials_are_rejected() {
        let response: Response = router()
            .oneshot(
                Request::get("/events?id=bad&token=bad")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
}
