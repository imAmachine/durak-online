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
use tokio::sync::{mpsc, RwLock};
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
    use tower::ServiceExt;

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
