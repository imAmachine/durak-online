pub mod events;
mod game;
pub mod storage;

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
}

impl AppState {
    pub fn with_repository(repository: storage::Repository) -> Self {
        Self {
            repository: Some(Arc::new(Mutex::new(repository))),
            ready: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
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
    router_with_state(AppState::default())
}

pub fn router_with_state(state: AppState) -> Router {
    Router::new()
        .route("/events", get(events))
        .route("/api/status", get(status))
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

async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Result<impl IntoResponse, Response> {
    let id = query.id.filter(|value| is_client_id(value));
    let token = query.token.filter(|value| is_token(value));
    let (Some(id), Some(token)) = (id, token) else {
        return Err(error(
            StatusCode::BAD_REQUEST,
            "Нужны корректные id и токен сессии",
        ));
    };

    {
        let mut sessions = state.sessions.write().await;
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
        let account = accounts
            .entry(id.clone())
            .or_insert_with(|| game::Account::new(&id, &name));
        account.name = name.clone();
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
    !value.is_empty()
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
    async fn readiness_requires_a_migrated_repository() {
        let directory = tempdir().unwrap();
        let repository = storage::Repository::open(
            directory.path().join("durak.sqlite"),
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations"),
        )
        .unwrap();
        let response: Response = router_with_state(AppState::with_repository(repository))
            .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let response: Response = router()
            .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
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
