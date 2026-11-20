use crate::seat::{mark_seen, Seat};
use reqwest::blocking::{Client, Response};
use serde_json::{json, Value};
use std::{
    env,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
fn base() -> String {
    env::var("DURAK_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:4173".into())
        .trim_end_matches('/')
        .into()
}
fn client() -> Client {
    Client::builder()
        .timeout(Duration::from_secs(65))
        .build()
        .unwrap()
}
pub(super) fn body(path: &str, data: &Value) -> Result<Value, String> {
    let r = client()
        .post(format!("{}{}", base(), path))
        .json(data)
        .send()
        .map_err(|e| e.without_url().to_string())?;
    let status = r.status();
    let v: Result<Value, _> = r.json();
    let v = v.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(v["error"].as_str().unwrap_or("HTTP error").into());
    }
    Ok(v)
}
pub(super) fn post_action(data: &Value, action: &str) -> Result<Value, String> {
    let response = client()
        .post(format!("{}/api/action", base()))
        .json(data)
        .send()
        .map_err(|error| error.without_url().to_string())?;
    let status = response.status();
    let value = response.json::<Value>().unwrap_or(Value::Null);
    if !status.is_success() || !value["error"].is_null() {
        let message = value["error"]
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| format!("HTTP {}", status.as_u16()));
        return Err(format!("Сервер отклонил {}: {}", action, message));
    }
    Ok(value)
}
pub(super) fn status() -> Result<Value, String> {
    client()
        .get(format!("{}/api/status", base()))
        .send()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .map_err(|e| e.to_string())
}
pub(super) fn start_sse(seat: Arc<Mutex<Seat>>) {
    let base = base();
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(run_sse(seat, base));
    });
}

async fn run_sse(seat: Arc<Mutex<Seat>>, base: String) {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    loop {
        let (id, token, name) = {
            let s = seat.lock().unwrap();
            if s.closed {
                break;
            }
            (s.id.clone(), s.token.clone(), s.name.clone())
        };
        let url = format!(
            "{}/events?id={}&token={}&name={}",
            base,
            urlencoding(&id),
            urlencoding(&token),
            urlencoding(&name)
        );
        let request = client.get(url).send();
        tokio::pin!(request);
        let result = loop {
            tokio::select! {
                result = &mut request => break Some(result),
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if seat.lock().unwrap().closed { break None; }
                }
            }
        };
        let Some(result) = result else { break };
        match result {
            Ok(mut response) if response.status().is_success() => {
                {
                    let mut s = seat.lock().unwrap();
                    if s.closed {
                        break;
                    }
                    s.streaming = true;
                    s.last_error = None;
                }
                let mut pending = Vec::new();
                let mut data = Vec::new();
                loop {
                    tokio::select! {
                        chunk = response.chunk() => {
                            let chunk = match chunk {
                                Ok(Some(chunk)) => chunk,
                                Ok(None) => {
                                    if !pending.is_empty() {
                                        if pending.last() == Some(&b'\r') { pending.pop(); }
                                        if let Ok(line) = std::str::from_utf8(&pending) {
                                            apply_sse_line(line, &mut data, &seat);
                                        }
                                    }
                                    break;
                                }
                                Err(_) => break,
                            };
                            pending.extend_from_slice(&chunk);
                            while let Some(end) = pending.iter().position(|&b| b == b'\n') {
                                let mut line: Vec<u8> = pending.drain(..=end).collect();
                                line.pop();
                                if line.last() == Some(&b'\r') { line.pop(); }
                                let Ok(line) = std::str::from_utf8(&line) else { break };
                                if !apply_sse_line(line, &mut data, &seat) { break; }
                            }
                            if seat.lock().unwrap().closed { break; }
                        }
                        _ = tokio::time::sleep(Duration::from_millis(100)) => {
                            if seat.lock().unwrap().closed { break; }
                        }
                    }
                }
                seat.lock().unwrap().streaming = false;
            }
            Ok(response) => {
                seat.lock().unwrap().last_error = Some(format!("SSE HTTP {}", response.status()))
            }
            Err(error) => seat.lock().unwrap().last_error = Some(error.without_url().to_string()),
        }
        if seat.lock().unwrap().closed {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

fn apply_sse_line(line: &str, data: &mut Vec<String>, seat: &Arc<Mutex<Seat>>) -> bool {
    let mut seat = seat.lock().unwrap();
    if seat.closed {
        return false;
    }
    if line.is_empty() {
        if !data.is_empty() {
            if let Ok(value) = serde_json::from_str::<Value>(&data.join("\n")) {
                seat.apply_event(value);
            }
            data.clear();
        }
    } else if let Some(value) = line.strip_prefix("data:") {
        data.push(value.strip_prefix(' ').unwrap_or(value).to_owned());
    } else if line == "data" {
        data.push(String::new());
    }
    true
}

#[cfg(test)]
fn read_sse(reader: impl std::io::BufRead, seat: &Arc<Mutex<Seat>>) {
    let mut data = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if !apply_sse_line(&line, &mut data, seat) {
            break;
        }
    }
}

fn opponent_events<'a>(
    items: impl Iterator<Item = &'a Value>,
    id: &str,
    since: u64,
    room: &Value,
) -> bool {
    items
        .filter(|item| item["type"] == "game" && item["state"]["room"]["id"] == *room)
        .flat_map(|item| item["state"]["log"].as_array().into_iter().flatten())
        .any(|event| {
            event["seq"].as_u64().is_some_and(|seq| seq > since)
                && event["actorId"].as_str().is_some_and(|actor| actor != id)
        })
}

fn apply_watch_state(seat: &mut Seat, value: &Value) {
    if value.is_object() && value["seq"].as_u64().unwrap_or(0) >= seat.seq {
        seat.update_game(value.clone());
    }
}

fn urlencoding(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() {
                (b as char).to_string()
            } else {
                format!("%{:02X}", b)
            }
        })
        .collect()
}
fn watch_request(
    id: &str,
    token: &str,
    since: u64,
    until: &str,
    timeout: u64,
) -> Result<Response, String> {
    let url = format!(
        "{}/api/watch?id={}&token={}&since={}&until={}&timeout={}",
        base(),
        urlencoding(id),
        urlencoding(token),
        since,
        urlencoding(until),
        timeout
    );
    client()
        .get(url)
        .timeout(Duration::from_millis(timeout.saturating_add(5000)))
        .send()
        .map_err(|e| e.without_url().to_string())
}
fn reached(state: Option<&Value>, until: &str) -> bool {
    let Some(state) = state else { return false };
    match until {
        "your-turn" => {
            !state["you"].is_null()
                && [
                    "canAttack",
                    "canDefend",
                    "canTransfer",
                    "canTake",
                    "canPass",
                    "canStart",
                ]
                .iter()
                .any(|key| state[*key] == true)
        }
        "finished" => {
            (!state["finished"].is_null() && state["finished"] != false)
                || state["room"]["status"] == "finished"
        }
        _ => false,
    }
}

fn cached_watch(seat: &Arc<Mutex<Seat>>, until: &str) -> Option<Value> {
    let mut s = seat.lock().unwrap();
    let fresh: Vec<_> = s
        .events
        .iter()
        .filter(|item| item["streamSeq"].as_u64().unwrap_or(0) > s.seen_stream_seq)
        .collect();
    let any_game = fresh.iter().any(|item| item["type"] == "game");
    let room = s
        .game
        .as_ref()
        .map(|state| &state["room"]["id"])
        .unwrap_or(&Value::Null);
    let since = s
        .events
        .iter()
        .filter(|item| {
            item["streamSeq"].as_u64().unwrap_or(0) <= s.seen_stream_seq
                && item["type"] == "game"
                && item["state"]["room"]["id"] == *room
        })
        .filter_map(|item| item["state"]["seq"].as_u64())
        .max()
        .unwrap_or(0);
    let hit = (!fresh.is_empty()
        && (until == "any"
            || (until == "game" && any_game)
            || (until == "opponent"
                && opponent_events(fresh.iter().copied(), &s.id, since, room))))
        || reached(s.game.as_ref(), until);
    if !hit {
        return None;
    }
    let mut events: Vec<Value> = fresh
        .iter()
        .flat_map(|item| {
            item["state"]["log"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned()
        })
        .collect();
    events.sort_by_key(|event| event["seq"].as_u64().unwrap_or(0));
    events.dedup_by_key(|event| event["seq"].as_u64().unwrap_or(0));
    let seq = s
        .game
        .as_ref()
        .and_then(|state| state["seq"].as_u64())
        .unwrap_or(0);
    let since = events
        .first()
        .and_then(|event| event["seq"].as_u64())
        .map(|seq| seq.saturating_sub(1))
        .unwrap_or(seq);
    let state = s.game.clone();
    let room_id = state
        .as_ref()
        .map(|state| state["room"]["id"].clone())
        .unwrap_or(Value::Null);
    s.seen_stream_seq = s.stream_seq;
    Some(
        json!({"via":"cache","hit":true,"seq":seq,"since":since,"events":events,"state":state,"roomId":room_id}),
    )
}

fn wait_sse(seat: &Arc<Mutex<Seat>>, until: &str, timeout: u64) -> Value {
    let (before, since, room) = {
        let s = seat.lock().unwrap();
        (
            s.stream_seq,
            s.seq,
            s.game
                .as_ref()
                .map(|state| state["room"]["id"].clone())
                .unwrap_or(Value::Null),
        )
    };
    let deadline = Instant::now() + Duration::from_millis(timeout);
    loop {
        let s = seat.lock().unwrap();
        let hit = s.stream_seq > before
            && match until {
                "any" => true,
                "game" => s.game.is_some(),
                "opponent" => opponent_events(
                    s.events
                        .iter()
                        .filter(|item| item["streamSeq"].as_u64().unwrap_or(0) > before),
                    &s.id,
                    since,
                    &room,
                ),
                _ => reached(s.game.as_ref(), until),
            };
        if hit || s.closed || Instant::now() >= deadline {
            let seq = s
                .game
                .as_ref()
                .and_then(|state| state["seq"].as_u64())
                .unwrap_or(0);
            let room_id = s
                .game
                .as_ref()
                .map(|state| state["room"]["id"].clone())
                .unwrap_or(Value::Null);
            return json!({"via":"sse","hit":hit,"seq":seq,"since":seq,"events":[],"state":s.game,"roomId":room_id});
        }
        drop(s);
        thread::sleep(
            Duration::from_millis(5).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

fn hook_result(seat: &Arc<Mutex<Seat>>, v: &Value, since: u64) -> Value {
    let mut s = seat.lock().unwrap();
    apply_watch_state(&mut s, &v["state"]);
    let events: Vec<Value> = v["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|event| event["seq"].as_u64().unwrap_or(0) > since)
        .cloned()
        .collect();
    json!({"via":"hook","hit":v["hit"] == true,"seq":v["seq"].as_u64().unwrap_or(0),
        "since":since,"roomId":v["roomId"],"events":events,
        "state":s.game.clone().unwrap_or(Value::Null)})
}

pub(super) fn wait_watch(
    seat: &Arc<Mutex<Seat>>,
    until: &str,
    timeout: u64,
) -> Result<Value, String> {
    if let Some(v) = cached_watch(seat, until) {
        return Ok(v);
    }
    if seat.lock().unwrap().game.is_none() {
        let result = wait_sse(seat, until, timeout);
        mark_seen(seat);
        return Ok(result);
    }
    let (id, token, since) = {
        let s = seat.lock().unwrap();
        (s.id.clone(), s.token.clone(), s.seq)
    };
    let response = watch_request(&id, &token, since, until, timeout);
    let result = match response {
        Err(_) => wait_sse(seat, until, timeout),
        Ok(r) if r.status().as_u16() == 404 => wait_sse(seat, until, timeout),
        Ok(r) => {
            let status = r.status();
            let v: Value = r.json().unwrap_or_else(|_| json!({}));
            if !status.is_success() {
                return Err(format!(
                    "/api/watch {}: {}",
                    status.as_u16(),
                    v["error"].as_str().unwrap_or("ошибка")
                ));
            }
            hook_result(seat, &v, since)
        }
    };
    mark_seen(seat);
    Ok(result)
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transport_shutdown_tests.rs"]
mod shutdown_tests;
