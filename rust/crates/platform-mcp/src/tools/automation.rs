use super::{action, connect, tool};
use crate::{
    presentation::{compact, result},
    seat::{save_sessions, seat_arg, seats, wait_seq, Seat},
    strategy::choose_move,
    transport::{body, wait_watch},
};
use serde_json::{json, Value};
use std::{
    env,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
fn checked_action(args: &Value, act: &str) -> Result<Value, String> {
    let response = action(args, act)?;
    if response["isError"] == true {
        return Err(response["content"][0]["text"]
            .as_str()
            .unwrap_or("Действие отклонено")
            .to_owned());
    }
    Ok(response)
}
fn wait_game_seq(seat: &Arc<Mutex<Seat>>, target: u64, timeout: Duration) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let before = {
            let s = seat.lock().unwrap();
            if s.seq >= target {
                return Ok(());
            }
            s.stream_seq
        };
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("Нет нового снимка SSE после действия".into());
        }
        wait_seq(seat, before, remaining)?;
    }
}
fn my_move(seat: &Arc<Mutex<Seat>>, st: &Value) -> Option<Value> {
    let s = seat.lock().unwrap();
    choose_move(st, Some(&s.ledger))
}
pub(super) fn follow(a: &Value) -> Result<Value, String> {
    let q = seat_arg(a)?;
    let (label, id) = {
        let s = q.lock().unwrap();
        (s.label.clone(), s.id.clone())
    };
    let until = a["until"].as_str().unwrap_or("finished");
    if until != "finished" && until != "turn" {
        return Err("until должен быть finished или turn".into());
    }
    if q.lock().unwrap().game.is_none() {
        let mut response = result(
            format!("{}: вне стола.", label),
            json!({"seat":label,"finished":false}),
        );
        response["isError"] = true.into();
        return Ok(response);
    }
    let max = a["maxSteps"]
        .as_u64()
        .filter(|n| *n > 0)
        .unwrap_or(400)
        .clamp(1, 2000);
    let duration = Duration::from_millis(
        a["totalMs"]
            .as_u64()
            .filter(|n| *n > 0)
            .unwrap_or(240000)
            .clamp(1000, 900000),
    );
    let start = Instant::now();
    let mut cursor = q.lock().unwrap().seen_stream_seq;
    let mut steps = 0;
    let mut spins = 0;
    let mut log = Vec::new();
    let mut logged_events = std::collections::HashSet::new();
    let reason = loop {
        if start.elapsed() >= duration {
            break "timeout";
        }
        let Some(st) = q.lock().unwrap().game.clone() else {
            break "no-room";
        };
        if st["room"]["status"] == "finished" || !st["finished"].is_null() {
            break "finished";
        }
        if steps >= max {
            break "steps";
        }
        if let Some(mut m) = my_move(&q, &st) {
            m["seat"] = label.clone().into();
            let act = m["action"].as_str().unwrap().to_string();
            if let Err(message) = checked_action(&m, &act) {
                log.push(format!("ошибка: {}", message));
                break "error";
            }
            steps += 1;
            let note = follow_move_text(&m);
            log.push(note);
            follow_replay_stream(&q, &id, &mut cursor, &mut log, &mut logged_events);
        } else {
            if until == "turn" {
                break "opponent";
            }
            let stream_before = q.lock().unwrap().stream_seq;
            let own_turn = follow_own_turn(&st);
            let condition = if st["room"]["status"] == "playing" && own_turn {
                "your-turn"
            } else {
                "any"
            };
            let cached = follow_cached(&q, condition, cursor);
            let events = if let Some(events) = cached {
                events
            } else {
                let left = duration.saturating_sub(start.elapsed()).as_millis() as u64;
                if left == 0 {
                    break "timeout";
                }
                let wait = wait_watch(&q, condition, left.clamp(300, 25000))?;
                wait["events"].as_array().cloned().unwrap_or_default()
            };
            let arrived = !events.is_empty() || q.lock().unwrap().stream_seq > stream_before;
            follow_replay(&events, &id, &mut log, &mut logged_events);
            follow_replay_stream(&q, &id, &mut cursor, &mut log, &mut logged_events);
            let current = q.lock().unwrap().game.clone();
            if start.elapsed() >= duration {
                break "timeout";
            }
            if !own_turn || !current.as_ref().is_some_and(follow_own_turn) {
                spins = 0;
                if !arrived {
                    std::thread::sleep(
                        Duration::from_millis(50).min(duration.saturating_sub(start.elapsed())),
                    );
                }
                continue;
            }
            if let Some(delay) = follow_backoff(&mut spins, arrived) {
                if delay.is_zero() {
                    break "no-move";
                }
                std::thread::sleep(delay.min(duration.saturating_sub(start.elapsed())));
            }
        }
    };
    if log.len() > 60 {
        log.drain(..log.len() - 60);
    }
    let (st, snapshot) = {
        let s = q.lock().unwrap();
        let st = s.game.clone().unwrap_or(Value::Null);
        (st.clone(), compact(&st, Some(&s.ledger)))
    };
    let finished = st["room"]["status"] == "finished" || !st["finished"].is_null();
    let text = format!(
        "Партия: {}, действий {}\nХоды:\n{}",
        reason,
        steps,
        log.iter()
            .rev()
            .take(25)
            .rev()
            .map(|line| format!("  {}", line))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let mut state = st.clone();
    if let Some(obj) = state.as_object_mut() {
        obj.remove("log");
    }
    Ok(result(
        text,
        json!({"seat":label,"roomId":st["room"]["id"],"steps":steps,"finished":finished,"reason":reason,"loser":st["finished"]["loser"],"payout":st["payout"].as_i64().unwrap_or(0),"waitedMs":start.elapsed().as_millis(),"actions":log,"snapshot":snapshot,"state":if a["verbose"]==true{state}else{Value::Null}}),
    ))
}

fn follow_own_turn(st: &Value) -> bool {
    if st["room"]["status"] != "playing" {
        return st["canStart"] == true;
    }
    if st["defenderId"] != st["you"]["id"]
        && !st["takePending"].as_bool().unwrap_or(false)
        && st["table"]
            .as_array()
            .is_some_and(|table| table.iter().any(|row| row["defense"].is_null()))
    {
        return false;
    }
    [
        "canAttack",
        "canDefend",
        "canTransfer",
        "canTake",
        "canPass",
    ]
    .iter()
    .any(|key| st[*key] == true)
}

fn follow_backoff(spins: &mut u32, arrived: bool) -> Option<Duration> {
    if arrived {
        *spins = 0;
        None
    } else {
        *spins += 1;
        Some(if *spins >= 3 {
            Duration::ZERO
        } else {
            Duration::from_millis(50 * u64::from(*spins))
        })
    }
}

fn follow_move_text(m: &Value) -> String {
    let action = m["action"].as_str().unwrap_or("");
    let card = m["cardId"]
        .as_str()
        .map(|v| format!(" {}", v))
        .unwrap_or_default();
    let target = m["targetCardId"]
        .as_str()
        .map(|v| format!(" на {}", v))
        .unwrap_or_default();
    let reason = m["reason"]
        .as_str()
        .map(|v| format!(" ({})", v))
        .unwrap_or_default();
    format!("я {}{}{}{}", action, card, target, reason)
}

fn follow_replay(
    events: &[Value],
    id: &str,
    log: &mut Vec<String>,
    seen: &mut std::collections::HashSet<u64>,
) {
    for event in events {
        if event["actorId"] == id {
            continue;
        }
        if let Some(seq) = event["seq"].as_u64() {
            if !seen.insert(seq) {
                continue;
            }
        }
        let action = event["action"].as_str().unwrap_or("undefined");
        let verb = match action {
            "attack" => "ходит",
            "defend" => "отбивает",
            "transfer" => "переводит",
            "take" => "берёт карты",
            "pass" => "говорит «бито»",
            "start" => "начинает партию",
            "create" => "создаёт стол",
            "join" => "садится за стол",
            "leave" => "покидает стол",
            "connect" => "подключается",
            "disconnect" => "отключается",
            "rename" => "меняет имя",
            "admin" => "правит профиль",
            "cancel" => "отменяет партию",
            "sync" => "обновляет стол",
            _ => action,
        };
        let who = event["actorName"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or("кто-то");
        let card = event["cardId"]
            .as_str()
            .map(|v| format!(" {}", v))
            .unwrap_or_default();
        let target = event["targetCardId"]
            .as_str()
            .map(|v| format!(" на {}", v))
            .unwrap_or_default();
        log.push(format!("{} {}{}{}", who, verb, card, target));
    }
}

fn follow_cached(q: &Arc<Mutex<Seat>>, condition: &str, cursor: u64) -> Option<Vec<Value>> {
    let s = q.lock().unwrap();
    let fresh: Vec<_> = s
        .events
        .iter()
        .filter(|event| event["streamSeq"].as_u64().unwrap_or(0) > cursor)
        .collect();
    if fresh.is_empty() {
        return None;
    }
    let ready = condition == "any"
        || s.game.as_ref().is_some_and(|st| {
            [
                "canAttack",
                "canDefend",
                "canTransfer",
                "canTake",
                "canPass",
                "canStart",
            ]
            .iter()
            .any(|key| st[*key] == true)
        });
    if !ready {
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
    Some(events)
}

fn follow_replay_stream(
    q: &Arc<Mutex<Seat>>,
    id: &str,
    cursor: &mut u64,
    log: &mut Vec<String>,
    seen: &mut std::collections::HashSet<u64>,
) {
    let mut s = q.lock().unwrap();
    let mut events: Vec<Value> = s
        .events
        .iter()
        .filter(|item| item["streamSeq"].as_u64().unwrap_or(0) > *cursor)
        .flat_map(|item| {
            item["state"]["log"]
                .as_array()
                .into_iter()
                .flatten()
                .cloned()
        })
        .collect();
    events.sort_by_key(|event| event["seq"].as_u64().unwrap_or(0));
    follow_replay(&events, id, log, seen);
    *cursor = s.stream_seq;
    s.seen_stream_seq = s.seen_stream_seq.max(*cursor);
}
struct AutoCleanup {
    labels: Vec<String>,
    keep: bool,
}
impl Drop for AutoCleanup {
    fn drop(&mut self) {
        if !self.keep {
            for label in &self.labels {
                if let Some(q) = seats().lock().unwrap().remove(label) {
                    q.lock().unwrap().closed = true;
                }
            }
            save_sessions();
        }
    }
}
fn credentials_for<F, C>(
    a: &Value,
    count: u64,
    get_env: F,
    mut create: C,
) -> Result<Vec<Value>, String>
where
    F: Fn(&str) -> Result<String, std::env::VarError>,
    C: FnMut(&str) -> Result<Value, String>,
{
    let explicit = a.get("credentials").is_some();
    let supplied = if explicit {
        a["credentials"].clone()
    } else if let Ok(raw) = get_env("DURAK_ACCOUNTS") {
        if raw.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&raw)
                .map_err(|_| "DURAK_ACCOUNTS должен содержать JSON-массив сессий")?
        }
    } else {
        let accounts: Value =
            serde_json::from_str(&get_env("DURAK_DISCORD_ACCOUNTS").unwrap_or_default())
                .unwrap_or(Value::Null);
        Value::Array(
            (1..=count)
                .map(|i| accounts[&format!("auto{}", i)].clone())
                .collect(),
        )
    };
    let mut credentials = supplied.as_array().cloned().unwrap_or_default();
    if !explicit
        && (credentials.len() < count as usize
            || (get_env("DURAK_ACCOUNTS").is_err() && credentials.iter().all(Value::is_null)))
    {
        let key = ["DURAK_ADMIN_KEY", "ADMIN_KEY"]
            .iter()
            .filter_map(|name| get_env(name).ok())
            .map(|value| value.trim().to_owned())
            .find(|value| !value.is_empty());
        if let Some(key) = key {
            credentials.clear();
            for _ in 0..count {
                let session = create(&key)?;
                credentials.push(session);
            }
        }
    }
    let mut ids = std::collections::HashSet::new();
    let mut tokens = std::collections::HashSet::new();
    if credentials.len() < count as usize {
        return Err("Для autoplay нужны отдельные credentials, DURAK_ACCOUNTS, сессии auto1...autoN в DURAK_DISCORD_ACCOUNTS или ключ администратора".into());
    }
    for account in credentials.iter().take(count as usize) {
        let id = account["clientId"].as_str().unwrap_or("");
        let token = account["token"].as_str().unwrap_or("");
        if id.trim().is_empty()
            || token.trim().is_empty()
            || !ids.insert(id.to_string())
            || !tokens.insert(token.to_string())
        {
            return Err("Для autoplay нужны отдельные credentials, DURAK_ACCOUNTS, сессии auto1...autoN в DURAK_DISCORD_ACCOUNTS или ключ администратора".into());
        }
    }
    Ok(credentials)
}
fn create_bot(key: &str) -> Result<Value, String> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(|_| "Не удалось создать данные аккаунта")?;
    let name = format!(
        "bot-{}",
        random[..8]
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );
    let password = format!(
        "pw-{}",
        random
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );
    let created = body(
        "/api/admin",
        &json!({"key":key,"op":"create_temp","name":name,"password":password}),
    )
    .map_err(|e| format!("Не удалось создать аккаунт бота: {}", e))?;
    let login = created["account"]["name"].as_str().unwrap_or(&name);
    body(
        "/api/auth/temp/login",
        &json!({"name":login,"password":password}),
    )
    .map_err(|e| format!("Не удалось войти ботом: {}", e))
}
pub(super) fn autoplay(a: &Value) -> Result<Value, String> {
    let count = a["seats"].as_u64().unwrap_or(2).clamp(2, 6);
    let credentials = credentials_for(a, count, |name| env::var(name), create_bot)?;
    let max = a["maxSteps"].as_u64().unwrap_or(300).clamp(10, 2000);
    let mut cleanup = AutoCleanup {
        labels: Vec::new(),
        keep: a["keepSeats"] == true,
    };
    for i in 1..=count {
        let base = format!("auto{}", i);
        let mut label = base.clone();
        let mut suffix = 1u64;
        while seats().lock().unwrap().contains_key(&label) {
            label = format!("{}_{}", base, suffix);
            suffix += 1;
        }
        let name = format!(
            "{} {}",
            a["name"]
                .as_str()
                .unwrap_or("Бот")
                .chars()
                .take(16)
                .collect::<String>(),
            i
        );
        connect(
            &json!({"seat":label,"name":name,"clientId":credentials[(i - 1) as usize]["clientId"],"token":credentials[(i - 1) as usize]["token"]}),
        )?;
        cleanup.labels.push(label);
    }
    let host = &cleanup.labels[0];
    let room = tool(
        "durak_room_create",
        &json!({"seat":host,"name":"MCP тест","mode":a["mode"],"deckSize":a["deckSize"],"bet":a["bet"]}),
    )?;
    let code = room["structuredContent"]["code"].clone();
    let mut seq = room["structuredContent"]["state"]["seq"]
        .as_u64()
        .ok_or("Нет номера состояния стола")?;
    for label in cleanup.labels.iter().skip(1) {
        let joined = tool("durak_room_join", &json!({"seat":label,"code":code}))?;
        seq = joined["structuredContent"]["state"]["seq"]
            .as_u64()
            .ok_or("Нет номера состояния стола")?;
    }
    for label in &cleanup.labels {
        wait_game_seq(
            &seat_arg(&json!({"seat":label}))?,
            seq,
            Duration::from_secs(3),
        )?;
    }
    checked_action(&json!({"seat":host}), "start")?;
    seq += 1;
    let mut log = Vec::new();
    let mut final_state = Value::Null;
    for _ in 0..max {
        for label in &cleanup.labels {
            let q = seat_arg(&json!({"seat":label}))?;
            wait_game_seq(&q, seq, Duration::from_secs(3))?;
        }
        let mut chosen = None;
        for label in &cleanup.labels {
            let q = seat_arg(&json!({"seat":label}))?;
            let st = q.lock().unwrap().game.clone().unwrap_or(Value::Null);
            if st["room"]["status"] == "finished" || !st["finished"].is_null() {
                final_state = st;
                break;
            }
            if let Some(m) = my_move(&q, &st) {
                chosen = Some((label.clone(), m));
                break;
            }
        }
        if !final_state.is_null() {
            break;
        }
        let (label, mut m) = chosen.ok_or("Боты не нашли допустимый ход")?;
        m["seat"] = label.clone().into();
        let act = m["action"].as_str().unwrap().to_string();
        checked_action(&m, &act)?;
        seq += 1;
        let q = seat_arg(&json!({"seat":label}))?;
        wait_game_seq(&q, seq, Duration::from_secs(3))?;
        let st = q.lock().unwrap().game.clone().unwrap_or(Value::Null);
        final_state = st;
        log.push(m);
        if final_state["room"]["status"] == "finished" || !final_state["finished"].is_null() {
            break;
        }
        final_state = Value::Null;
    }
    if final_state.is_null() {
        final_state = seat_arg(&json!({"seat":host}))?
            .lock()
            .unwrap()
            .game
            .clone()
            .unwrap_or(Value::Null);
    }
    let finished =
        final_state["room"]["status"] == "finished" || !final_state["finished"].is_null();
    let balances:Vec<Value>=cleanup.labels.iter().map(|l|{let q=seat_arg(&json!({"seat":l})).unwrap();let s=q.lock().unwrap();json!({"seat":l,"balance":s.account["balance"],"payout":s.game.as_ref().map(|v|v["payout"].clone())})}).collect();
    Ok(result(
        format!(
            "Стол {}: {}",
            code,
            if finished {
                "партия завершена"
            } else {
                "лимит ходов"
            }
        ),
        json!({"code":code,"seats":cleanup.labels,"steps":log.len(),"finished":finished,"loser":final_state["finished"]["loser"],"deck":final_state["deck"],"table":final_state["table"].as_array().map(|v|v.len()),"hands":final_state["players"].as_array().map(|v|v.iter().map(|p|p["cards"].as_u64().unwrap_or(0)).sum::<u64>()),"balances":balances,"actions":log,"state":final_state}),
    ))
}
#[cfg(test)]
mod follow_tests {
    use super::*;

    static URL_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn autoplay_barrier_waits_for_applied_action() {
        let seat = Arc::new(Mutex::new(Seat::default()));
        seat.lock()
            .unwrap()
            .apply_event(json!({"type":"game","state":{"seq":10,"room":{"id":"r"}}}));
        assert!(wait_game_seq(&seat, 11, Duration::from_millis(5)).is_err());
        seat.lock()
            .unwrap()
            .apply_event(json!({"type":"game","state":{"seq":11,"room":{"id":"r"}}}));
        assert!(wait_game_seq(&seat, 11, Duration::from_millis(5)).is_ok());
    }

    #[test]
    fn replay_collects_stream_and_hook_events_once() {
        let q = Arc::new(Mutex::new(Seat {
            id: "me".into(),
            ..Default::default()
        }));
        let opponent =
            json!({"seq":4,"actorId":"other","actorName":"Бот","action":"attack","cardId":"6♠"});
        let own = json!({"seq":5,"actorId":"me","action":"pass"});
        {
            let mut s = q.lock().unwrap();
            s.apply_event(json!({"type":"game","state":{"seq":5,"room":{"status":"playing"},"canPass":true,"log":[opponent,own]}}));
            s.seen_stream_seq = s.stream_seq;
        }
        let mut cursor = 0;
        let mut log = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let events = follow_cached(&q, "your-turn", cursor).unwrap();
        follow_replay(&events, "me", &mut log, &mut seen);
        follow_replay_stream(&q, "me", &mut cursor, &mut log, &mut seen);
        assert_eq!(log, vec!["Бот ходит 6♠"]);
        assert!(follow_cached(&q, "your-turn", cursor).is_none());
    }

    #[test]
    fn no_move_backoff_only_applies_to_own_turn() {
        let waiting = json!({"room":{"status":"playing"},"canAttack":false,"canDefend":false,"canTransfer":false,"canTake":false,"canPass":false,"canStart":false});
        let own = json!({"room":{"status":"playing"},"canAttack":true});
        assert!(!follow_own_turn(&waiting));
        assert!(follow_own_turn(&own));
        let optional_attack = json!({"room":{"status":"playing"},"you":{"id":"me"},"defenderId":"other","canAttack":true,"table":[{"attack":{"id":"6♠"},"defense":null}]});
        assert!(!follow_own_turn(&optional_attack));
    }

    #[test]
    fn no_move_backoff_resets_after_arrival_and_stops_after_three() {
        let mut spins = 0;
        assert_eq!(
            follow_backoff(&mut spins, false),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            follow_backoff(&mut spins, false),
            Some(Duration::from_millis(100))
        );
        assert_eq!(follow_backoff(&mut spins, true), None);
        assert_eq!(
            follow_backoff(&mut spins, false),
            Some(Duration::from_millis(50))
        );
        assert_eq!(
            follow_backoff(&mut spins, false),
            Some(Duration::from_millis(100))
        );
        assert_eq!(follow_backoff(&mut spins, false), Some(Duration::ZERO));
    }

    #[test]
    fn follow_falls_back_on_404_and_replays_sse() {
        let _guard = URL_LOCK.lock().unwrap();
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let label = "follow_404_fallback_test";
        let state = json!({"seq":1,"room":{"id":"r","status":"playing"},"you":{"id":"me"},"hand":[],"table":[],"finished":null});
        let seat = Arc::new(Mutex::new(Seat {
            label: label.into(),
            id: "me".into(),
            token: "token".into(),
            game: Some(state),
            seq: 1,
            ..Default::default()
        }));
        seats().lock().unwrap().insert(label.into(), seat.clone());
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("watch connection: {error}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buf = [0; 4096];
            let count = stream.read(&mut buf).unwrap();
            assert!(String::from_utf8_lossy(&buf[..count]).contains("GET /api/watch?"));
            stream
                .write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            std::thread::sleep(Duration::from_millis(50));
            seat.lock().unwrap().apply_event(json!({"type":"game","state":{"seq":2,"room":{"id":"r","status":"finished"},"you":{"id":"me"},"canPass":true,"finished":{"loser":"other"},"log":[{"seq":2,"actorId":"other","actorName":"Бот","action":"attack","cardId":"6♠"}]}}));
        });
        let old = std::env::var_os("DURAK_URL");
        std::env::set_var("DURAK_URL", format!("http://{}", addr));
        let response = follow(&json!({"seat":label,"totalMs":1000})).unwrap();
        seats().lock().unwrap().remove(label);
        if let Some(value) = old {
            std::env::set_var("DURAK_URL", value);
        } else {
            std::env::remove_var("DURAK_URL");
        }
        server.join().unwrap();
        assert_eq!(response["structuredContent"]["reason"], "finished");
        assert_eq!(
            response["structuredContent"]["actions"],
            json!(["Бот ходит 6♠"])
        );
    }

    #[test]
    fn rejected_action_does_not_count_as_a_move() {
        let _guard = URL_LOCK.lock().unwrap();
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let _ = stream.read(&mut buf).unwrap();
            let payload = r#"{"error":"move rejected"}"#;
            write!(stream, "HTTP/1.1 409 Conflict\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", payload.len(), payload).unwrap();
        });
        let old = std::env::var_os("DURAK_URL");
        std::env::set_var("DURAK_URL", format!("http://{}", addr));
        let label = "follow_rejected_action_test";
        let state = json!({"seq":1,"room":{"status":"playing"},"you":{"id":"me"},"attackerId":"me","defenderId":"other","hand":[{"id":"6♠","rank":"6","suit":"♠"}],"table":[],"canAttack":true,"finished":null});
        seats().lock().unwrap().insert(
            label.into(),
            Arc::new(Mutex::new(Seat {
                label: label.into(),
                id: "me".into(),
                token: "token".into(),
                game: Some(state),
                ..Default::default()
            })),
        );
        let response = follow(&json!({"seat":label})).unwrap();
        seats().lock().unwrap().remove(label);
        if let Some(value) = old {
            std::env::set_var("DURAK_URL", value);
        } else {
            std::env::remove_var("DURAK_URL");
        }
        server.join().unwrap();
        assert_eq!(response["structuredContent"]["steps"], 0);
        assert_eq!(response["structuredContent"]["reason"], "error");
        assert!(response["structuredContent"]["actions"][0]
            .as_str()
            .unwrap()
            .contains("move rejected"));
    }

    #[test]
    fn missing_room_returns_mcp_error() {
        let label = "follow_missing_room_test";
        seats().lock().unwrap().insert(
            label.into(),
            Arc::new(Mutex::new(Seat {
                label: label.into(),
                ..Default::default()
            })),
        );
        let response = follow(&json!({"seat":label})).unwrap();
        seats().lock().unwrap().remove(label);
        assert_eq!(response["isError"], true);
        assert_eq!(response["structuredContent"]["finished"], false);
    }

    #[test]
    fn follows_permissions_for_other_attackers() {
        let mut state = json!({"room":{"status":"playing"},"you":{"id":"c","finished":false,"left":false},"attackerId":"a","defenderId":"b","takePending":false,"table":[{"attack":{"id":"6♠","rank":"6","suit":"♠"},"defense":{"id":"7♠","rank":"7","suit":"♠"}}],"hand":[{"id":"6♥","rank":"6","suit":"♥"}],"trump":"♣","canAttack":true,"canDefend":false,"canPass":true});
        assert_eq!(choose_move(&state, None).unwrap()["action"], "attack");
        state["canAttack"] = json!(false);
        assert_eq!(choose_move(&state, None).unwrap()["action"], "pass");
        state["takePending"] = json!(true);
        assert_eq!(choose_move(&state, None).unwrap()["action"], "pass");
        state["takePending"] = json!(false);
        state["canAttack"] = json!(false);
        state["canPass"] = json!(false);
        assert!(choose_move(&state, None).is_none());
    }
}

#[cfg(test)]
mod autoplay_tests {
    use super::*;
    use std::collections::HashMap;

    fn select(args: Value, vars: &[(&str, &str)]) -> Result<Vec<Value>, String> {
        let vars: HashMap<_, _> = vars.iter().copied().collect();
        let mut n = 0;
        credentials_for(
            &args,
            2,
            |key| {
                vars.get(key)
                    .map(|v| v.to_string())
                    .ok_or(env::VarError::NotPresent)
            },
            |key| {
                n += 1;
                Ok(json!({"clientId":format!("{}-{}",key,n),"token":format!("token-{}",n)}))
            },
        )
    }

    #[test]
    fn credentials_sources_and_admin_fallback() {
        let sessions = json!([{"clientId":"a","token":"x"},{"clientId":"b","token":"y"}]);
        let raw = sessions.to_string();
        assert_eq!(
            select(
                json!({"credentials":sessions}),
                &[("DURAK_ACCOUNTS", "bad")]
            )
            .unwrap(),
            sessions.as_array().unwrap().clone()
        );
        assert_eq!(
            select(json!({}), &[("DURAK_ACCOUNTS", &raw)]).unwrap(),
            sessions.as_array().unwrap().clone()
        );
        assert!(select(
            json!({}),
            &[("DURAK_ACCOUNTS", "bad"), ("ADMIN_KEY", "key")]
        )
        .is_err());
        let fallback =
            select(json!({}), &[("DURAK_ADMIN_KEY", " "), ("ADMIN_KEY", "key")]).unwrap();
        assert_eq!(fallback[0]["clientId"], "key-1");
        assert_eq!(fallback[1]["clientId"], "key-2");
        assert!(select(
            json!({"credentials":[{"clientId":"a","token":"x"},{"clientId":"a","token":"y"}]}),
            &[("ADMIN_KEY", "key")]
        )
        .is_err());
        assert!(select(json!({}), &[]).is_err());
        let discord = json!({"auto1":sessions[0],"auto2":sessions[1]}).to_string();
        assert_eq!(
            select(json!({}), &[("DURAK_DISCORD_ACCOUNTS", &discord)]).unwrap(),
            sessions.as_array().unwrap().clone()
        );
    }

    #[test]
    fn explicit_incomplete_credentials_never_create_accounts() {
        for credentials in [
            json!([]),
            json!([{"clientId":"a","token":"x"}]),
            json!({}),
            Value::Null,
        ] {
            let mut calls = 0;
            let result = credentials_for(
                &json!({"credentials": credentials}),
                2,
                |name| {
                    if name == "ADMIN_KEY" {
                        Ok("key".into())
                    } else {
                        Err(env::VarError::NotPresent)
                    }
                },
                |_| {
                    calls += 1;
                    Ok(json!({}))
                },
            );
            assert!(result.is_err());
            assert_eq!(calls, 0);
        }
    }

    #[test]
    fn waits_for_game_sequence_not_unrelated_stream_events() {
        let seat = Arc::new(Mutex::new(Seat::default()));
        seat.lock()
            .unwrap()
            .apply_event(json!({"type":"game","state":{"seq":1}}));
        seat.lock()
            .unwrap()
            .apply_event(json!({"type":"account","account":{}}));
        assert!(wait_game_seq(&seat, 2, Duration::from_millis(10)).is_err());
        seat.lock()
            .unwrap()
            .apply_event(json!({"type":"game","state":{"seq":2}}));
        assert!(wait_game_seq(&seat, 2, Duration::from_millis(10)).is_ok());
    }
}
