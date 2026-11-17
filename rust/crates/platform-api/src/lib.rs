use axum::{
    extract::{Query, State},
    http::{header, StatusCode, Uri},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{any, get},
    Json, Router,
};
use futures_core::Stream;
use serde_json::{json, Value};
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

const START_BALANCE: i64 = 1000;

type SseResult = Result<Event, Infallible>;

type ClientSender = mpsc::Sender<SseResult>;

struct ClientEntry {
    generation: u64,
    _sender: ClientSender,
}

#[derive(Clone, Default)]
pub struct AppState {
    sessions: Arc<RwLock<HashMap<String, String>>>,
    clients: Arc<RwLock<HashMap<String, ClientEntry>>>,
    rooms: Arc<RwLock<HashMap<String, Value>>>,
    next_client_generation: Arc<AtomicU64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
struct EventsQuery {
    id: Option<String>,
    token: Option<String>,
    name: Option<String>,
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

    let name = query.name.unwrap_or_else(|| "Игрок".into());
    let account = json!({
        "id": id.clone(),
        "name": name,
        "balance": START_BALANCE,
        "games": 0,
        "wins": 0,
        "losses": 0,
        "online": true
    });
    let lobby = json!({"type":"lobby", "online": state.clients.read().await.len() + 1, "rooms": [], "history": []});
    let noroom = json!({"type":"noroom"});
    let retry = Event::default().retry(Duration::from_secs(2));
    let (sender, receiver) = mpsc::channel(4);
    let generation = state.next_client_generation.fetch_add(1, Ordering::Relaxed);
    {
        let mut clients = state.clients.write().await;
        clients.insert(
            id.clone(),
            ClientEntry {
                generation,
                _sender: sender.clone(),
            },
        );
    }
    for event in [
        retry,
        Event::default().data(json!({"type":"account", "account": account}).to_string()),
        Event::default().data(noroom.to_string()),
        Event::default().data(lobby.to_string()),
    ] {
        let _ = sender.send(Ok(event)).await;
    }
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
    let clients = state.clients.read().await.len();
    let rooms = state.rooms.read().await;
    Json(json!({"online": clients, "clients": [], "rooms": rooms.values().collect::<Vec<_>>() }))
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
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
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
