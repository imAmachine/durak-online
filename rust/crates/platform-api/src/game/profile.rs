use super::{
    auth, broadcast_lobby, commit_room, error, is_client_id, publish_room, push_account, Account,
    AppState, CommitMeta,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::{json, Value};

pub async fn profile(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return *e,
    };
    let requested = body.get("name").and_then(Value::as_str).map(str::trim);
    let name = match requested {
        Some(name) if !name.is_empty() => name,
        Some(_) => return error(StatusCode::BAD_REQUEST, "Имя не может быть пустым"),
        None => {
            let accounts = state.accounts.read().await;
            let account = accounts
                .get(&id)
                .cloned()
                .unwrap_or_else(|| Account::new(&id, "Игрок"));
            return Json(json!({"ok":true,"name":account.name,"account":account.public(true)}))
                .into_response();
        }
    };
    let mut rooms = state.rooms.write().await;
    let mut accounts = state.accounts.write().await;
    let mut next_accounts = accounts.clone();
    let account = next_accounts
        .entry(id.clone())
        .or_insert_with(|| Account::new(&id, "Игрок"));
    account.name = name.chars().take(20).collect();
    let out = json!({"ok":true,"name":account.name,"account":account.public(true)});
    if let Some(r) = rooms
        .values_mut()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        let mut next_room = r.clone();
        if commit_room(
            &state,
            &mut next_room,
            &accounts,
            &next_accounts,
            CommitMeta {
                actor: &id,
                action: "rename",
                data: &body,
            },
            false,
        )
        .await
        .is_err()
        {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
        *r = next_room;
    } else if let Some(repository) = &state.repository {
        if repository
            .lock()
            .await
            .save_account(next_accounts.get(&id).unwrap())
            .is_err()
        {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
    }
    *accounts = next_accounts;
    drop(accounts);
    if let Some(c) = state.clients.write().await.get_mut(&id) {
        c.name = name.chars().take(20).collect();
    }
    push_account(&state, &id).await;
    if let Some(r) = rooms
        .values()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        publish_room(&state, r).await;
    }
    broadcast_lobby(&state, &rooms).await;
    (StatusCode::OK, Json(out)).into_response()
}
pub async fn player_profile(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(id) => id,
        Err(response) => return response.into_response(),
    };
    let Some(room_id) = body.get("roomId").and_then(Value::as_str) else {
        return error(StatusCode::BAD_REQUEST, "Нужен код стола");
    };
    let Some(player_id) = body
        .get("playerId")
        .and_then(Value::as_str)
        .filter(|id| is_client_id(id))
    else {
        return error(StatusCode::BAD_REQUEST, "Нужен идентификатор игрока");
    };
    let rooms = state.rooms.read().await;
    let can_view = rooms.get(room_id).is_some_and(|room| {
        room.state
            .players
            .iter()
            .any(|player| player.id == id && !player.left)
            && room
                .state
                .players
                .iter()
                .any(|player| player.id == player_id)
    });
    if !can_view {
        return error(StatusCode::NOT_FOUND, "Игрок за этим столом не найден");
    }
    let accounts = state.accounts.read().await;
    let Some(account) = accounts.get(player_id) else {
        return error(StatusCode::NOT_FOUND, "Профиль игрока не найден");
    };
    Json(json!({
        "balance": account.balance,
        "games": account.games,
        "wins": account.wins,
        "losses": account.losses,
    }))
    .into_response()
}
