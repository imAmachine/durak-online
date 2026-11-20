mod automation;

use crate::{
    presentation::{compact, format_state, hint_payload, result, wait_result, without_log},
    seat::{mark_seen, seat_arg, seats, wait_seq, Seat},
    strategy::choose_move,
    transport::{body, post_action, start_sse, status, wait_watch},
};
use automation::{autoplay, follow};
use serde_json::{json, Value};
use std::{
    env,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};
fn connect(a: &Value) -> Result<Value, String> {
    let label = a
        .get("seat")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let label = if label.is_empty() {
        format!("p{}", seats().lock().unwrap().len() + 1)
    } else {
        label
    };
    let name = a
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or(&format!("MCP {}", label))
        .chars()
        .take(20)
        .collect::<String>();
    let (session_id, session_token) = if a["create"] == true {
        let key = a
            .get("key")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| env::var("DURAK_ADMIN_KEY").ok())
            .or_else(|| env::var("ADMIN_KEY").ok())
            .filter(|v| !v.trim().is_empty())
            .ok_or("Создание аккаунта требует key, DURAK_ADMIN_KEY или ADMIN_KEY")?;
        let mut random = [0u8; 24];
        getrandom::fill(&mut random).map_err(|_| "Не удалось создать данные аккаунта")?;
        let account_name = format!(
            "bot-{}",
            random[..8]
                .iter()
                .map(|byte| format!("{:02x}", byte))
                .collect::<String>()
        );
        let password = random
            .iter()
            .map(|byte| format!("{:02x}", byte))
            .collect::<String>();
        let created = body(
            "/api/admin",
            &json!({"key":key,"op":"create_temp","name":account_name,"password":password}),
        )?;
        let login = created["account"]["name"]
            .as_str()
            .ok_or("Сервис не вернул имя созданного аккаунта")?;
        let session = body(
            "/api/auth/temp/login",
            &json!({"name":login,"password":password}),
        )?;
        (
            session["clientId"].as_str().unwrap_or("").to_owned(),
            session["token"].as_str().unwrap_or("").to_owned(),
        )
    } else if a.get("login").and_then(Value::as_str).is_some()
        || (a.get("clientId").is_none()
            && a.get("token").is_none()
            && env::var("DURAK_LOGIN")
                .ok()
                .filter(|v| !v.trim().is_empty())
                .is_some())
    {
        let login = a
            .get("login")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| env::var("DURAK_LOGIN").ok())
            .unwrap_or_default();
        let password = a
            .get("password")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| env::var("DURAK_PASSWORD").ok())
            .unwrap_or_default();
        let login = login.trim();
        if login.is_empty() || password.is_empty() {
            return Err(
                "Укажите login и password клубного аккаунта или DURAK_LOGIN и DURAK_PASSWORD"
                    .into(),
            );
        }
        let session = body(
            "/api/auth/temp/login",
            &json!({"name":login,"password":password}),
        )?;
        (
            session["clientId"].as_str().unwrap_or("").to_owned(),
            session["token"].as_str().unwrap_or("").to_owned(),
        )
    } else {
        let accounts: Value =
            serde_json::from_str(&env::var("DURAK_DISCORD_ACCOUNTS").unwrap_or_default())
                .unwrap_or(Value::Null);
        let selected = accounts.as_object().and_then(|v| v.get(&label));
        let id = a
            .get("clientId")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| selected.and_then(|v| v["clientId"].as_str()).map(str::to_owned))
            .or_else(|| env::var("DURAK_CLIENT_ID").ok())
            .or_else(|| env::var("DURAK_DISCORD_CLIENT_ID").ok())
            .filter(|v| !v.trim().is_empty())
            .ok_or("Укажите login/password, clientId/token готовой сессии, DURAK_LOGIN/DURAK_PASSWORD, DURAK_CLIENT_ID/DURAK_TOKEN или DURAK_DISCORD_CLIENT_ID и DURAK_DISCORD_TOKEN")?;
        let token = a
            .get("token")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| selected.and_then(|v| v["token"].as_str()).map(str::to_owned))
            .or_else(|| env::var("DURAK_TOKEN").ok())
            .or_else(|| env::var("DURAK_DISCORD_TOKEN").ok())
            .filter(|v| !v.trim().is_empty())
            .ok_or("Укажите login/password, clientId/token готовой сессии, DURAK_LOGIN/DURAK_PASSWORD, DURAK_CLIENT_ID/DURAK_TOKEN или DURAK_DISCORD_CLIENT_ID и DURAK_DISCORD_TOKEN")?;
        (id, token)
    };
    if session_id.trim().is_empty() || session_token.trim().is_empty() {
        return Err("Сервис не вернул clientId и token для сессии".into());
    }
    let id = session_id;
    let token = session_token;
    let seat = Arc::new(Mutex::new(Seat {
        label: label.clone(),
        name,
        id,
        token,
        ..Default::default()
    }));
    {
        let mut m = seats().lock().unwrap();
        if m.contains_key(&label) {
            return Err("Место уже открыто".into());
        }
        m.insert(label.clone(), seat.clone());
    }
    crate::seat::save_sessions();
    start_sse(seat.clone());
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if seat.lock().unwrap().streaming {
            let payload_until = Instant::now() + Duration::from_secs(5);
            loop {
                let s = seat.lock().unwrap();
                if s.game.is_some()
                    || s.lobby.is_some()
                    || s.noroom
                    || Instant::now() >= payload_until
                {
                    return Ok(result(
                        format!("Открыто место {}.", label),
                        json!({"seat":label,"name":s.name,"clientId":s.id,"connected":s.streaming,"account":s.account,"room":s.game.as_ref().map(|g| &g["room"]),"status":s.game.as_ref().map(|g| g["room"]["status"].clone()).unwrap_or_else(|| if s.noroom { json!("lobby") } else { json!("connecting") }),"lastError":s.last_error}),
                    ));
                }
                drop(s);
                thread::sleep(Duration::from_millis(20));
            }
        }
        thread::sleep(Duration::from_millis(20));
    }
    seat.lock().unwrap().closed = true;
    seats().lock().unwrap().remove(&label);
    crate::seat::save_sessions();
    Err("Не удалось открыть SSE поток".into())
}
fn action(a: &Value, act: &str) -> Result<Value, String> {
    let seat = seat_arg(a)?;
    let (id, token, before) = {
        let s = seat.lock().unwrap();
        (s.id.clone(), s.token.clone(), s.stream_seq)
    };
    let mut b = json!({"clientId":id,"token":token,"action":act});
    for k in ["cardId", "targetCardId"] {
        if let Some(v) = a.get(k) {
            b[k] = v.clone()
        }
    }
    let v = match post_action(&b, act) {
        Ok(value) => value,
        Err(message) => {
            let label = seat.lock().unwrap().label.clone();
            let data = if act == "start" {
                json!({"seat":label})
            } else {
                json!({"seat":label,"action":act})
            };
            let mut response = result(message, data);
            response["isError"] = true.into();
            return Ok(response);
        }
    };
    let applied = if let Some(st) = v.get("state").filter(|state| state.is_object()) {
        let seq = st["seq"].as_u64().unwrap_or(0);
        let mut current = seat.lock().unwrap();
        if seq >= current.seq {
            current.update_game(st.clone());
            true
        } else {
            false
        }
    } else {
        false
    };
    if !applied {
        if let Err(message) = wait_seq(&seat, before, Duration::from_secs(3)) {
            let label = seat.lock().unwrap().label.clone();
            let data = if act == "start" {
                json!({"seat":label})
            } else {
                json!({"seat":label,"action":act})
            };
            let mut response = result(format!("Действие принято, но {}", message), data);
            response["isError"] = true.into();
            return Ok(response);
        }
    }
    let s = seat.lock().unwrap();
    let st = s.game.clone().unwrap_or(Value::Null);
    let verbose = a["verbose"] == true;
    let mut data = json!({"seat":s.label,"snapshot":compact(&st, Some(&s.ledger))});
    if act == "start" || verbose {
        data["state"] = without_log(&st);
    }
    if act != "start" {
        data["action"] = json!(act);
        data["account"] = s.account.clone();
    }
    let text = format!(
        "{}\n\n{}",
        if act == "start" {
            "Партия началась.".to_owned()
        } else {
            format!("{}: принято.", act)
        },
        format_state(&s, &st, verbose)
    );
    drop(s);
    mark_seen(&seat);
    Ok(result(text, data))
}
fn hints(a: &Value) -> Result<Value, String> {
    let q = seat_arg(a)?;
    let s = q.lock().unwrap();
    let st = s.game.as_ref().ok_or("Место вне стола")?;
    let data = hint_payload(&s, st);
    drop(s);
    mark_seen(&q);
    Ok(result("Подсказки".into(), data))
}
fn turn(a: &Value) -> Result<Value, String> {
    let q = seat_arg(a)?;
    let s = q.lock().unwrap();
    let st = s.game.as_ref().ok_or("Место вне стола")?;
    let mut data = hint_payload(&s, st);
    let recommended = choose_move(st, Some(&s.ledger)).unwrap_or(Value::Null);
    let text = match recommended["action"].as_str() {
        Some(action) => format!(
            "Рекомендую: {}{} — {}",
            action,
            recommended["cardId"]
                .as_str()
                .map(|c| format!(" {}", c))
                .unwrap_or_default(),
            recommended["reason"].as_str().unwrap_or("")
        ),
        None => "Ход не ваш: ждите через durak_watch (until=your-turn).".into(),
    };
    if let Some(obj) = data.as_object_mut() {
        obj.insert("recommended".into(), recommended);
        obj.insert("snapshot".into(), compact(st, Some(&s.ledger)));
        if a["verbose"].as_bool().unwrap_or(false) {
            obj.insert("state".into(), without_log(st));
        }
        obj.insert("text".into(), json!(text));
    }
    drop(s);
    mark_seen(&q);
    Ok(result("Ход".into(), data))
}
pub(super) fn tool(name: &str, a: &Value) -> Result<Value, String> {
    match name {
        "durak_connect" => connect(a),
        "durak_seats" => {
            let m = seats().lock().unwrap();
            let list:Vec<Value>=m.values().map(|q|{let s=q.lock().unwrap();json!({"seat":s.label,"name":s.name,"clientId":s.id,"connected":s.streaming,"account":s.account,"room":s.game.as_ref().map(|g|g["room"].clone()),"lastError":s.last_error})}).collect();
            Ok(result("Открытые места".into(), json!({"seats":list})))
        }
        "durak_status" => {
            let v = status()?;
            Ok(result("Статус сервиса".into(), v))
        }
        "durak_lobby" => {
            let lobby = if let Ok(q) = seat_arg(a) {
                q.lock().unwrap().lobby.clone()
            } else {
                None
            };
            let v = match lobby {
                Some(v) => v,
                None => status()?,
            };
            Ok(result("Лобби".into(), v))
        }
        "durak_disconnect" | "durak_room_leave" => {
            let q = seat_arg(a)?;
            let (id, token, label) = {
                let s = q.lock().unwrap();
                (s.id.clone(), s.token.clone(), s.label.clone())
            };
            if name == "durak_room_leave" || a["leave"] == true {
                body("/api/leave", &json!({"clientId":id,"token":token}))?;
                q.lock().unwrap().clear_room();
            }
            if name == "durak_disconnect" {
                q.lock().unwrap().closed = true;
                seats().lock().unwrap().remove(&label);
                crate::seat::save_sessions();
            }
            Ok(result("Готово".into(), json!({"seat":label})))
        }
        "durak_room_create" | "durak_room_join" | "durak_profile" => {
            let q = seat_arg(a)?;
            let (id, token, player, label) = {
                let s = q.lock().unwrap();
                (
                    s.id.clone(),
                    s.token.clone(),
                    s.name.clone(),
                    s.label.clone(),
                )
            };
            let mut b = json!({"clientId":id,"token":token,"playerName":player});
            let path = if name == "durak_room_create" {
                for k in ["name", "mode", "deckSize", "bet"] {
                    if let Some(v) = a.get(k) {
                        b[k] = v.clone()
                    }
                }
                "/api/rooms"
            } else if name == "durak_room_join" {
                b["roomId"] = a.get("code").cloned().ok_or("Укажите code")?;
                "/api/join"
            } else {
                b["name"] = a.get("name").cloned().ok_or("Укажите name")?;
                "/api/profile"
            };
            let v = body(path, &b)?;
            if name == "durak_profile" {
                q.lock().unwrap().name = a["name"].as_str().unwrap_or("").into();
            } else {
                for _ in 0..100 {
                    if q.lock()
                        .unwrap()
                        .game
                        .as_ref()
                        .map(|g| g["room"]["id"] == v["room"]["id"])
                        .unwrap_or(false)
                    {
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            }
            let (st, snapshot) = {
                let s = q.lock().unwrap();
                let st = s.game.clone().unwrap_or(Value::Null);
                (st.clone(), compact(&st, Some(&s.ledger)))
            };
            Ok(result(
                "Готово".into(),
                json!({"seat":label,"code":v["room"]["id"],"room":v["room"],"account":v["account"],"snapshot":snapshot,"state":st}),
            ))
        }
        "durak_start" => action(a, "start"),
        "durak_action" => {
            let act = a["action"].as_str().unwrap_or("");
            if !["attack", "defend", "transfer", "take", "pass"].contains(&act) {
                return Err("Неверное действие".into());
            }
            action(a, act)
        }
        "durak_state" => {
            let q = seat_arg(a)?;
            let s = q.lock().unwrap();
            let st = s.game.clone().unwrap_or(Value::Null);
            let data = json!({"seat":s.label,"snapshot":compact(&st, Some(&s.ledger)),"state":st});
            drop(s);
            mark_seen(&q);
            Ok(result("Состояние стола".into(), data))
        }
        "durak_turn" => turn(a),
        "durak_wait" | "durak_watch" => {
            let q = seat_arg(a)?;
            let default = if name == "durak_watch" {
                "your-turn"
            } else {
                "any"
            };
            let until = a["until"].as_str().unwrap_or(default);
            if !["any", "game", "your-turn", "opponent", "finished"].contains(&until) {
                return Err("Неверный until".into());
            }
            let total = if name == "durak_watch" {
                a["totalMs"].as_u64().unwrap_or(180000).clamp(200, 600000)
            } else {
                a["timeoutMs"].as_u64().unwrap_or(5000).clamp(200, 120000)
            };
            let chunk = a["chunkMs"].as_u64().unwrap_or(25000).clamp(200, 60000);
            let start = Instant::now();
            let mut events = Vec::new();
            let mut first_since = None;
            loop {
                let rem = total.saturating_sub(start.elapsed().as_millis() as u64);
                let v = wait_watch(&q, until, rem.min(chunk).max(200))?;
                if first_since.is_none() {
                    first_since = v.get("since").cloned();
                }
                events.extend(v["events"].as_array().into_iter().flatten().cloned());
                if v["hit"] == true
                    || start.elapsed().as_millis() as u64 >= total
                    || v["roomId"].is_null()
                {
                    let mut combined = v;
                    combined["events"] = json!(events);
                    if let Some(since) = first_since {
                        combined["since"] = since;
                    }
                    let s = q.lock().unwrap();
                    return Ok(wait_result(
                        &s,
                        until,
                        &combined,
                        start.elapsed().as_millis() as u64,
                        a["verbose"] == true,
                    ));
                }
            }
        }
        "durak_admin" => {
            let mut b = a.clone();
            if b["key"].is_null() {
                b["key"] = env::var("DURAK_ADMIN_KEY")
                    .or_else(|_| env::var("ADMIN_KEY"))
                    .unwrap_or_default()
                    .into()
            }
            let v = body("/api/admin", &b)?;
            Ok(result("Администрирование".into(), v))
        }
        "durak_hints" => hints(a),
        "durak_follow" => follow(a),
        "durak_autoplay" => autoplay(a),
        _ => Err("Неизвестный инструмент".into()),
    }
}
