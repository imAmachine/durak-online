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
        Self { seq: 0, events: Vec::new(), changes }
    }
}

impl EventHub {
    pub async fn publish(&self, room_id: impl Into<String>, payload: Value) -> EventRecord {
        let mut rooms = self.rooms.write().await;
        let room = rooms.entry(room_id.into()).or_default();
        room.seq = payload.get("seq").and_then(Value::as_u64).filter(|seq| *seq > room.seq).unwrap_or(room.seq + 1);
        let record = EventRecord { seq: room.seq, payload };
        room.events.push(record.clone());
        if room.events.len() > 40 { room.events.remove(0); }
        room.changes.send_replace(room.seq);
        record
    }

    pub async fn since(&self, room_id: &str, since: u64) -> (u64, Vec<EventRecord>) {
        let rooms = self.rooms.read().await;
        match rooms.get(room_id) {
            Some(room) => (room.seq, room.events.iter().filter(|event| event.seq > since).cloned().collect()),
            None => (0, Vec::new()),
        }
    }

    pub async fn wait_for_change(&self, room_id: &str, since: u64) -> (u64, Vec<EventRecord>) {
        let mut changes = {
            let mut rooms = self.rooms.write().await;
            rooms.entry(room_id.to_owned()).or_default().changes.subscribe()
        };
        loop {
            let result = self.since(room_id, since).await;
            if result.0 > since { return result; }
            if changes.changed().await.is_err() { return result; }
        }
    }
}
