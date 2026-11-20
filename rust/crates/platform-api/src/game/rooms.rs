use super::{
    auth, broadcast_lobby, commit_room, error, publish_room, push_snapshot, room_summary, Account,
    AppState, CommitMeta, Room, BETS, ROOM_NAME_LIMIT,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use durak_engine::{DeckSize, DurakState, Mode};
use serde_json::{json, Value};
use std::collections::HashMap;

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return *e,
    };
    let clients = state.clients.read().await;
    let Some(client) = clients.get(&id) else {
        return error(StatusCode::BAD_REQUEST, "Нет соединения с сервером");
    };
    let requested_name = body
        .get("playerName")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| name.chars().take(20).collect::<String>());
    let current_name = state
        .accounts
        .read()
        .await
        .get(&id)
        .map(|account| account.name.clone())
        .or(requested_name)
        .unwrap_or_else(|| client.name.clone());
    drop(clients);
    let mut rooms = state.rooms.write().await;
    if rooms
        .values()
        .any(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        return error(StatusCode::BAD_REQUEST, "Вы уже в лобби");
    };
    let mut random = [0u8; 3];
    let rid = loop {
        if getrandom::getrandom(&mut random).is_err() {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Ошибка генератора случайных чисел",
            );
        };
        let code = format!("{:02X}{:02X}{:02X}", random[0], random[1], random[2]);
        if !rooms.contains_key(&code) {
            if let Some(repository) = &state.repository {
                match repository.lock().await.load_transition_state(&code) {
                    Ok(None) => break code,
                    Ok(Some(_)) => continue,
                    Err(_) => {
                        return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно")
                    }
                }
            } else {
                break code;
            }
        }
    };
    let mode = if body.get("mode").and_then(Value::as_str) == Some("transfer") {
        Mode::Transfer
    } else {
        Mode::Classic
    };
    let size = if body.get("deckSize").and_then(Value::as_u64) == Some(52) {
        DeckSize::FiftyTwo
    } else {
        DeckSize::ThirtySix
    };
    let requested = body
        .get("bet")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|x| x.parse().ok()))
        })
        .unwrap_or(0);
    let bet = if BETS.contains(&requested) {
        requested
    } else {
        0
    };
    let mut accounts = state.accounts.write().await;
    let original_accounts = accounts.clone();
    let account = accounts
        .entry(id.clone())
        .or_insert_with(|| Account::new(&id, &current_name));
    if account.balance < bet {
        return error(StatusCode::BAD_REQUEST, "Недостаточно фишек");
    };
    let account = account.public(true);
    let mut r = Room {
        id: rid.clone(),
        name: body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Новая партия")
            .chars()
            .take(ROOM_NAME_LIMIT)
            .collect(),
        mode,
        deck_size: size,
        bet,
        host: id.clone(),
        state: DurakState::new(vec![id.clone()], mode, size),
        seq: 0,
        events: vec![],
        pot: 0,
        payouts: HashMap::new(),
        message: "Ожидаем игроков. Отправьте друзьям код приглашения".into(),
    };
    let connected = state.clients.read().await.keys().cloned().collect();
    let updated_accounts = accounts.clone();
    if commit_room(
        &state,
        &mut r,
        &original_accounts,
        &updated_accounts,
        CommitMeta {
            actor: &id,
            action: "create",
            data: &body,
        },
        true,
    )
    .await
    .is_err()
    {
        *accounts = original_accounts;
        return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
    }
    let summary = room_summary(&r, &accounts, &connected);
    drop(accounts);
    publish_room(&state, &r).await;
    rooms.insert(rid.clone(), r);
    broadcast_lobby(&state, &rooms).await;
    (
        StatusCode::CREATED,
        Json(json!({"room":summary,"code":rid,"account":account})),
    )
        .into_response()
}
pub async fn join(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return *e,
    };
    if !state.clients.read().await.contains_key(&id) {
        return error(StatusCode::BAD_REQUEST, "Нет соединения с сервером");
    };
    let rid = body
        .get("roomId")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_uppercase();
    let mut rooms = state.rooms.write().await;
    let Some(target) = rooms.get(&rid) else {
        return error(StatusCode::NOT_FOUND, "Лобби с таким кодом не найдено");
    };
    let prior = target.state.players.iter().position(|p| p.id == id);
    let existing = prior.is_some_and(|index| !target.state.players[index].left);
    if target.state.status == durak_engine::Status::Playing && !existing {
        return error(StatusCode::BAD_REQUEST, "Игра уже началась");
    };
    if prior.is_none() && target.state.players.len() >= 6 {
        return error(StatusCode::BAD_REQUEST, "В лобби нет свободных мест");
    };
    if !existing
        && state
            .accounts
            .read()
            .await
            .get(&id)
            .is_some_and(|a| a.balance < target.bet)
    {
        return error(StatusCode::BAD_REQUEST, "Недостаточно фишек");
    };
    let previous = rooms
        .iter()
        .find(|(key, r)| *key != &rid && r.state.players.iter().any(|p| p.id == id && !p.left))
        .map(|(key, _)| key.clone());
    if previous.is_some() {
        return error(StatusCode::BAD_REQUEST, "Сначала покиньте текущее лобби");
    }
    let current = rooms.get_mut(&rid).unwrap();
    let mut next_room = current.clone();
    let r = &mut next_room;
    let mut accounts = state.accounts.write().await;
    let mut next_accounts = accounts.clone();
    if !next_accounts.contains_key(&id) {
        let stored = if let Some(repository) = &state.repository {
            match repository.lock().await.load_account(&id) {
                Ok(account) => account,
                Err(_) => return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно"),
            }
        } else {
            None
        };
        if let Some(account) = stored {
            accounts.insert(id.clone(), account.clone());
            next_accounts.insert(id.clone(), account);
        } else {
            let requested = body
                .get("playerName")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned);
            let name = match requested {
                Some(name) => name,
                None => state
                    .clients
                    .read()
                    .await
                    .get(&id)
                    .map(|c| c.name.clone())
                    .unwrap_or_else(|| "Игрок".into()),
            };
            next_accounts.insert(
                id.clone(),
                Account::new(&id, &name.chars().take(20).collect::<String>()),
            );
        }
    }
    if !existing {
        if let Some(index) = prior {
            r.state.players[index].left = false;
        } else {
            r.state.players.push(durak_engine::Player {
                id: id.clone(),
                hand: vec![],
                finished: false,
                left: false,
            });
        }
    }
    r.message = "Игроки готовы. Создатель лобби может начать игру".into();
    let connected = state.clients.read().await.keys().cloned().collect();
    let out = json!({"room":room_summary(r,&next_accounts,&connected),"account":next_accounts.get(&id).map(|a|a.public(true))});
    if commit_room(
        &state,
        r,
        &accounts,
        &next_accounts,
        CommitMeta {
            actor: &id,
            action: "join",
            data: &body,
        },
        false,
    )
    .await
    .is_err()
    {
        return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
    }
    *current = next_room;
    *accounts = next_accounts;
    drop(accounts);
    publish_room(&state, current).await;
    push_snapshot(&state, current, &id).await;
    broadcast_lobby(&state, &rooms).await;
    (StatusCode::OK, Json(out)).into_response()
}
