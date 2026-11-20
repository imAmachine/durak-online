use crate::strategy::Ledger;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

#[derive(Default)]
pub(super) struct Seat {
    pub(super) label: String,
    pub(super) name: String,
    pub(super) id: String,
    pub(super) token: String,
    pub(super) streaming: bool,
    pub(super) closed: bool,
    pub(super) account: Value,
    pub(super) game: Option<Value>,
    pub(super) lobby: Option<Value>,
    pub(super) noroom: bool,
    pub(super) last_error: Option<String>,
    pub(super) seq: u64,
    pub(super) ledger: Ledger,
    pub(super) stream_seq: u64,
    pub(super) seen_stream_seq: u64,
    pub(super) events: Vec<Value>,
}

impl Seat {
    pub(super) fn clear_room(&mut self) {
        self.game = None;
        self.seq = 0;
        self.noroom = true;
        self.ledger = Ledger::default();
    }

    pub(super) fn update_game(&mut self, game: Value) {
        self.seq = game["seq"].as_u64().unwrap_or(self.seq);
        self.ledger.observe(&game);
        self.noroom = false;
        self.game = Some(game);
    }

    pub(super) fn apply_event(&mut self, mut event: Value) {
        self.stream_seq += 1;
        event["streamSeq"] = self.stream_seq.into();
        self.events.push(event.clone());
        if self.events.len() > 40 {
            self.events.remove(0);
        }
        match event["type"].as_str() {
            Some("game") => {
                let state = &event["state"];
                let same_room = self
                    .game
                    .as_ref()
                    .and_then(|game| game["room"]["id"].as_str())
                    .zip(state["room"]["id"].as_str())
                    .is_some_and(|(current, incoming)| current == incoming);
                let incoming_seq = state["seq"].as_u64().unwrap_or(0);
                if !same_room || incoming_seq >= self.seq {
                    self.update_game(state.clone());
                }
            }
            Some("account") => self.account = event["account"].clone(),
            Some("lobby") => self.lobby = Some(event),
            Some("noroom") => self.clear_room(),
            _ => {}
        }
    }
}
pub(super) type Seats = Arc<Mutex<HashMap<String, Arc<Mutex<Seat>>>>>;
static SEATS: OnceLock<Seats> = OnceLock::new();
fn session_path() -> PathBuf {
    env::var_os("DURAK_MCP_SESSIONS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".local/state/durak-mcp/sessions.json")
        })
}

static SESSION_TEMP_SEQ: AtomicU64 = AtomicU64::new(0);
static SESSION_SAVE_LOCK: Mutex<()> = Mutex::new(());

fn session_file_mode(file: &fs::File) {
    #[cfg(unix)]
    {
        let _ = file.set_permissions(fs::Permissions::from_mode(0o600));
    }
}

fn session_path_mode(path: &Path) {
    #[cfg(unix)]
    {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

fn write_sessions(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let sequence = SESSION_TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let temporary = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("sessions"),
        std::process::id(),
        sequence
    ));
    let result = (|| {
        let mut file = OpenOptions::new();
        file.write(true).create_new(true);
        #[cfg(unix)]
        file.mode(0o600);
        let mut file = file.open(&temporary)?;
        session_file_mode(&file);
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        session_path_mode(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub(super) fn save_sessions() {
    let _save_lock = SESSION_SAVE_LOCK.lock().unwrap();
    let list: Vec<Value> = seats()
        .lock()
        .unwrap()
        .values()
        .map(|seat| {
            let seat = seat.lock().unwrap();
            json!({"label":seat.label,"name":seat.name,"id":seat.id,"token":seat.token})
        })
        .collect();
    let path = session_path();
    let Ok(bytes) = serde_json::to_vec_pretty(&list) else {
        return;
    };
    let _ = write_sessions(&path, &bytes);
}

pub(super) fn restore_sessions() {
    let path = session_path();
    let Ok(bytes) = fs::read(&path) else { return };
    session_path_mode(&path);
    let Ok(items) = serde_json::from_slice::<Vec<Value>>(&bytes) else {
        return;
    };
    let mut stored = seats().lock().unwrap();
    for item in items {
        let Some(label) = item["label"].as_str().filter(|v| !v.is_empty()) else {
            continue;
        };
        let Some(id) = item["id"].as_str().filter(|v| !v.is_empty()) else {
            continue;
        };
        let Some(token) = item["token"].as_str().filter(|v| !v.is_empty()) else {
            continue;
        };
        if stored.contains_key(label) {
            continue;
        }
        stored.insert(
            label.to_owned(),
            Arc::new(Mutex::new(Seat {
                label: label.to_owned(),
                name: item["name"].as_str().unwrap_or(label).to_owned(),
                id: id.to_owned(),
                token: token.to_owned(),
                ..Default::default()
            })),
        );
    }
}

pub(super) fn seats() -> &'static Seats {
    SEATS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

pub(super) fn seat_arg(a: &Value) -> Result<Arc<Mutex<Seat>>, String> {
    let label = a.get("seat").and_then(Value::as_str).unwrap_or("");
    let m = seats().lock().unwrap();
    if !label.is_empty() {
        if let Some(s) = m.get(label) {
            return Ok(s.clone());
        }
        return Err(format!("Нет места «{}»", label));
    }
    if m.len() == 1 {
        return Ok(m.values().next().unwrap().clone());
    }
    if m.is_empty() {
        return Err("Нет открытых мест: сначала durak_connect".into());
    }
    Err("Открыто несколько мест: укажите seat".into())
}

pub(super) fn mark_seen(seat: &Arc<Mutex<Seat>>) {
    let mut s = seat.lock().unwrap();
    s.seen_stream_seq = s.stream_seq;
}

pub(super) fn wait_seq(
    seat: &Arc<Mutex<Seat>>,
    before: u64,
    timeout: Duration,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let s = seat.lock().unwrap();
            let current = s.game.as_ref();
            if s.events.iter().any(|event| {
                if event["streamSeq"].as_u64().unwrap_or(0) <= before || event["type"] != "game" {
                    return false;
                }
                let state = &event["state"];
                let Some(current) = current else { return true };
                state["room"]["id"] == current["room"]["id"]
                    && state["seq"].as_u64() == current["seq"].as_u64()
            }) {
                return Ok(());
            }
            if s.closed {
                return Err("Место закрыто".into());
            }
        }
        if Instant::now() >= deadline {
            return Err("Нет нового снимка SSE после действия".into());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod session_storage_tests_20261120 {
    use super::{wait_seq, write_sessions, Seat};
    use serde_json::json;
    use std::{
        fs,
        sync::{Arc, Mutex},
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn game(seq: u64, room: &str) -> serde_json::Value {
        json!({"type":"game","state":{"seq":seq,"room":{"id":room}}})
    }

    #[test]
    fn apply_event_rejects_older_same_room_game_20261120() {
        let mut seat = Seat::default();
        seat.apply_event(game(10, "room-a"));
        seat.apply_event(game(9, "room-a"));
        assert_eq!(seat.seq, 10);
        assert_eq!(seat.game.as_ref().unwrap()["seq"], 10);
    }

    #[test]
    fn apply_event_accepts_lower_seq_for_new_room_20261120() {
        let mut seat = Seat::default();
        seat.apply_event(game(10, "room-a"));
        seat.apply_event(game(1, "room-b"));
        assert_eq!(seat.seq, 1);
        assert_eq!(seat.game.as_ref().unwrap()["room"]["id"], "room-b");
    }

    #[test]
    fn wait_seq_ignores_stale_game_and_accepts_new_room_20261120() {
        let seat = Arc::new(Mutex::new(Seat::default()));
        seat.lock().unwrap().apply_event(game(10, "room-a"));
        let before = seat.lock().unwrap().stream_seq;
        seat.lock().unwrap().apply_event(game(9, "room-a"));
        assert!(wait_seq(&seat, before, Duration::from_millis(1)).is_err());
        seat.lock().unwrap().apply_event(game(1, "room-b"));
        let before = seat.lock().unwrap().stream_seq - 1;
        assert!(wait_seq(&seat, before, Duration::from_millis(1)).is_ok());
    }

    #[test]
    fn private_atomic_session_replacement_20261120() {
        let path = std::env::temp_dir().join(format!(
            "durak-session-test-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"old").unwrap();
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_sessions(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let prefix = format!(
            ".{}.{}.",
            path.file_name().unwrap().to_str().unwrap(),
            std::process::id()
        );
        assert!(fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(Result::ok)
            .all(|entry| !entry.file_name().to_string_lossy().starts_with(&prefix)));
        let _ = fs::remove_file(path);
    }
}
