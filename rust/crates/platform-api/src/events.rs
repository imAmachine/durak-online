use crate::{
    error, game, is_client_id, is_token, oauth, seeded_test_identity, verified_identity, AppState,
    ClientEntry, SseResult,
};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
};
use futures_core::Stream;
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    sync::atomic::Ordering,
    task::{Context, Poll},
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use serde_json::Value;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{watch, RwLock};

#[derive(Clone, Debug, PartialEq)]
pub struct EventRecord {
    pub seq: u64,
    pub payload: Value,
}

#[derive(Clone, Default)]
pub struct EventHub {
    rooms: Arc<RwLock<HashMap<String, RoomEvents>>>,
}

struct RoomEvents {
    seq: u64,
    events: Vec<EventRecord>,
    changes: watch::Sender<u64>,
}

impl Default for RoomEvents {
    fn default() -> Self {
        let (changes, _) = watch::channel(0);
        Self {
            seq: 0,
            events: Vec::new(),
            changes,
        }
    }
}

impl EventHub {
    pub async fn publish(&self, room_id: impl Into<String>, payload: Value) -> EventRecord {
        let mut rooms = self.rooms.write().await;
        let room = rooms.entry(room_id.into()).or_default();
        room.seq = payload
            .get("seq")
            .and_then(Value::as_u64)
            .filter(|seq| *seq > room.seq)
            .unwrap_or(room.seq + 1);
        let record = EventRecord {
            seq: room.seq,
            payload,
        };
        room.events.push(record.clone());
        if room.events.len() > 40 {
            room.events.remove(0);
        }
        room.changes.send_replace(room.seq);
        record
    }

    pub async fn since(&self, room_id: &str, since: u64) -> (u64, Vec<EventRecord>) {
        let rooms = self.rooms.read().await;
        match rooms.get(room_id) {
            Some(room) => (
                room.seq,
                room.events
                    .iter()
                    .filter(|event| event.seq > since)
                    .cloned()
                    .collect(),
            ),
            None => (0, Vec::new()),
        }
    }

    pub async fn wait_for_change(&self, room_id: &str, since: u64) -> (u64, Vec<EventRecord>) {
        let mut changes = {
            let mut rooms = self.rooms.write().await;
            rooms
                .entry(room_id.to_owned())
                .or_default()
                .changes
                .subscribe()
        };
        loop {
            let result = self.since(room_id, since).await;
            if result.0 > since {
                return result;
            }
            if changes.changed().await.is_err() {
                return result;
            }
        }
    }
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct EventsQuery {
    id: Option<String>,
    token: Option<String>,
    name: Option<String>,
}

struct ClientStream {
    inner: ReceiverStream<SseResult>,
    state: AppState,
    id: String,
    generation: u64,
    cancel: Option<tokio::sync::oneshot::Receiver<()>>,
}

impl Stream for ClientStream {
    type Item = SseResult;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Some(cancel) = &mut self.cancel {
            match Pin::new(cancel).poll(cx) {
                Poll::Ready(Ok(())) => return Poll::Ready(None),
                Poll::Ready(Err(_)) => self.cancel = None,
                Poll::Pending => {}
            }
        }
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

pub(crate) async fn events(
    State(state): State<AppState>,
    Query(query): Query<EventsQuery>,
) -> Response {
    let id = query
        .id
        .as_deref()
        .filter(|value| is_client_id(value))
        .map(str::to_owned);
    let token = query
        .token
        .as_deref()
        .filter(|value| is_token(value))
        .map(str::to_owned);
    let (Some(id), Some(token)) = (id, token) else {
        return error(
            StatusCode::BAD_REQUEST,
            "Нужны корректные id и токен сессии",
        );
    };

    {
        let mut sessions = state.sessions.write().await;
        let Some(repository) = &state.repository else {
            return error(StatusCode::FORBIDDEN, "Неверный токен сессии");
        };
        let verified_id = if cfg!(test) && seeded_test_identity(&id) {
            format!("discord:{id}")
        } else {
            id.clone()
        };
        if verified_id.starts_with("discord:")
            && state.oauth.is_none()
            && !(cfg!(test) && seeded_test_identity(&id))
        {
            return error(StatusCode::FORBIDDEN, "Неверный токен сессии");
        }
        match verified_identity(repository, &verified_id, &oauth::digest(&token)).await {
            Ok(true) => {}
            Ok(false) => return error(StatusCode::FORBIDDEN, "Неверный токен сессии"),
            Err(()) => return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно"),
        }
        if let Some(previous) = sessions.get(&id) {
            if previous != &token {
                return error(StatusCode::FORBIDDEN, "Неверный токен сессии");
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
                    Err(_) => {
                        return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно")
                    }
                }
            } else {
                None
            };
            accounts.insert(
                id.clone(),
                stored.unwrap_or_else(|| game::Account::new(&id, &name)),
            );
        }
        let account = accounts.get_mut(&id).unwrap();
        if let Some(repository) = &state.repository {
            if repository.lock().await.save_account(account).is_err() {
                return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
            }
        }
        account.public(true)
    };
    let retry = Event::default().retry(Duration::from_secs(2));
    let (sender, receiver) = mpsc::channel(128);
    let (cancel, cancelled) = tokio::sync::oneshot::channel();
    let generation = state.next_client_generation.fetch_add(1, Ordering::Relaxed);
    {
        let mut clients = state.clients.write().await;
        clients.insert(
            id.clone(),
            ClientEntry {
                generation,
                sender: sender.clone(),
                name,
                cancel: std::sync::Mutex::new(Some(cancel)),
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
        cancel: Some(cancelled),
    };
    Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

#[cfg(test)]
mod stream_overflow_tests {
    use super::*;

    fn client(capacity: usize, state: AppState, generation: u64) -> (ClientEntry, ClientStream) {
        let (sender, receiver) = mpsc::channel(capacity);
        let (cancel, cancelled) = tokio::sync::oneshot::channel();
        (
            ClientEntry {
                generation,
                sender,
                name: String::new(),
                cancel: std::sync::Mutex::new(Some(cancel)),
            },
            ClientStream {
                inner: ReceiverStream::new(receiver),
                state,
                id: "player".into(),
                generation,
                cancel: Some(cancelled),
            },
        )
    }

    #[tokio::test]
    async fn overflow_ends_stream_before_buffered_events_and_cleans_up() {
        let state = AppState::default();
        let (entry, mut stream) = client(1, state.clone(), 1);
        entry.send(Event::default().data("first"));
        entry.send(Event::default().data("overflow"));
        state.clients.write().await.insert("player".into(), entry);
        assert!(
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                .await
                .is_none()
        );
        drop(stream);
        tokio::task::yield_now().await;
        assert!(state.clients.read().await.is_empty());
    }

    #[tokio::test]
    async fn normal_delivery_and_replacement_survive_old_cleanup() {
        let state = AppState::default();
        let (old, mut stream) = client(1, state.clone(), 1);
        old.send(Event::default().data("first"));
        assert!(
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                .await
                .is_some()
        );
        old.send(Event::default().data("queued"));
        old.send(Event::default().data("overflow"));
        let (new, mut new_stream) = client(1, state.clone(), 2);
        new.send(Event::default().data("current"));
        state.clients.write().await.insert("player".into(), new);
        assert!(
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                .await
                .is_none()
        );
        drop(stream);
        tokio::task::yield_now().await;
        assert_eq!(
            state
                .clients
                .read()
                .await
                .get("player")
                .map(|c| c.generation),
            Some(2)
        );
        assert!(
            std::future::poll_fn(|cx| Pin::new(&mut new_stream).poll_next(cx))
                .await
                .is_some()
        );
    }

    #[tokio::test]
    async fn dropped_sender_delivers_queued_event_before_closing() {
        let (entry, mut stream) = client(1, AppState::default(), 1);
        entry.send(Event::default().data("revoked"));
        drop(entry);
        assert!(
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                .await
                .is_some()
        );
        assert!(
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx))
                .await
                .is_none()
        );
    }
}
