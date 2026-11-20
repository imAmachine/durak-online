use super::super::{oauth, password};
use super::{
    auth, broadcast_lobby, error, leave_inner, push_account, Account, AppState, START_BALANCE,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::{json, Value};

pub async fn admin(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if body.get("clientId").is_some() {
        if let Err(e) = auth(&state, &body).await {
            return *e;
        }
    }
    let key = std::env::var("ADMIN_KEY").unwrap_or_default();
    if key.trim().is_empty() {
        return error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Панель администратора отключена",
        );
    };
    if body.get("key").and_then(Value::as_str) != Some(key.trim()) {
        return error(StatusCode::FORBIDDEN, "Неверный ключ администратора");
    };
    let op = body.get("op").and_then(Value::as_str).unwrap_or("list");
    if op == "list" {
        let accounts = state.accounts.read().await;
        let clients = state.clients.read().await;
        return Json(json!({"ok":true,"players":accounts.values().map(|a|admin_view(a, clients.contains_key(&a.id))).collect::<Vec<_>>()})).into_response();
    };
    if op == "create_temp" {
        let name = body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let password = match body.get("password").and_then(Value::as_str) {
            Some(password) if password::is_valid(password) => password.to_owned(),
            Some(_) => {
                return error(
                    StatusCode::BAD_REQUEST,
                    "Пароль должен содержать от 4 до 128 символов",
                )
            }
            None => match oauth::random() {
                Ok(password) => password,
                Err(response) => return response.into_response(),
            },
        };
        if name.is_empty() || name.chars().count() > 20 {
            return error(StatusCode::BAD_REQUEST, "Укажите имя до 20 символов");
        }
        let mut accounts = state.accounts.write().await;
        if accounts
            .values()
            .any(|account| account.name.eq_ignore_ascii_case(name))
        {
            return error(StatusCode::CONFLICT, "Такое имя уже занято");
        }
        let id = match oauth::random() {
            Ok(value) => format!("temp:{value}"),
            Err(response) => return response.into_response(),
        };
        let mut account = Account::new(&id, name);
        account.password_hash = Some(password::hash(&password));
        if let Some(repository) = &state.repository {
            if repository
                .lock()
                .await
                .insert_temp_account(&account, "pending")
                .is_err()
            {
                return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
            }
        }
        let out = json!({"id":account.id,"name":account.name,"password":password,"createdAt":account.created_at});
        accounts.insert(id, account);
        return Json(json!({"ok":true,"account":out})).into_response();
    };
    let id = body.get("id").and_then(Value::as_str).unwrap_or("");
    if id.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Укажите игрока");
    };
    let mut accounts = state.accounts.write().await;
    let Some(stored) = accounts.get(id).cloned() else {
        return error(StatusCode::NOT_FOUND, "Игрок не найден");
    };
    if op == "delete" {
        drop(accounts);
        let room_ids: Vec<String> = state
            .rooms
            .read()
            .await
            .iter()
            .filter_map(|(rid, room)| {
                (room.state.status == durak_engine::Status::Playing
                    && room.state.players.iter().any(|p| p.id == id && !p.left))
                .then_some(rid.clone())
            })
            .collect();
        let mut rooms = state.rooms.write().await;
        for rid in room_ids {
            if leave_inner(&state, &mut rooms, &rid, id, true)
                .await
                .is_err()
            {
                return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
            }
        }
        broadcast_lobby(&state, &rooms).await;
        let live_rooms = rooms.keys().cloned().collect();
        if let Some(repository) = &state.repository {
            if repository
                .lock()
                .await
                .delete_account(id, &live_rooms)
                .is_err()
            {
                return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
            }
        }
        state.accounts.write().await.remove(id);
        drop(rooms);
        state.sessions.write().await.remove(id);
        state.clients.write().await.remove(id);
        return Json(json!({"ok":true,"deleted":stored.id,"name":stored.name})).into_response();
    };
    let mut candidates: Vec<Account> = accounts
        .values()
        .filter(|account| password::is_temporary(&account.id) && account.password_hash.is_some())
        .cloned()
        .collect();
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    let mut updated = stored.clone();
    let a = &mut updated;
    let mut revoke_session = false;
    match op {
        "reset" => {
            a.balance = START_BALANCE;
            a.games = 0;
            a.wins = 0;
            a.losses = 0;
        }
        "balance" => {
            let delta = body.get("delta").and_then(Value::as_i64).unwrap_or(0);
            if delta == 0 {
                return error(StatusCode::BAD_REQUEST, "Укажите сумму пополнения");
            };
            a.balance = a.balance.saturating_add(delta).clamp(0, 1_000_000_000);
        }
        "update" => {
            if let Some(name) = body.get("name").and_then(Value::as_str) {
                let name = name.trim();
                if name.is_empty() {
                    return error(StatusCode::BAD_REQUEST, "Имя не может быть пустым");
                };
                if accounts
                    .values()
                    .any(|other| other.id != id && other.name.eq_ignore_ascii_case(name))
                {
                    return error(StatusCode::CONFLICT, "Такое имя уже занято");
                }
                a.name = name.chars().take(20).collect();
            }
            if let Some(password) = body.get("password").and_then(Value::as_str) {
                if !password::is_temporary(&a.id) {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "Пароль можно менять только у временной учётки",
                    );
                }
                if !password::is_valid(password) {
                    return error(
                        StatusCode::BAD_REQUEST,
                        "Пароль должен содержать от 4 до 128 символов",
                    );
                }
                a.password_hash = Some(password::hash(password));
                revoke_session = true;
            }
            for (key, field) in [
                ("balance", &mut a.balance),
                ("games", &mut a.games),
                ("wins", &mut a.wins),
                ("losses", &mut a.losses),
            ] {
                if let Some(v) = body.get(key).and_then(Value::as_i64) {
                    *field = v.clamp(0, 1_000_000_000);
                }
            }
        }
        _ => return error(StatusCode::BAD_REQUEST, "Неизвестная операция"),
    };
    if let Some(repository) = &state.repository {
        if repository.lock().await.save_account(a).is_err() {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
        if revoke_session
            && repository
                .lock()
                .await
                .replace_password(id, a.password_hash.as_deref().unwrap_or_default(), true)
                .is_err()
        {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
    }
    let out = admin_view(a, state.clients.read().await.contains_key(id));
    accounts.insert(id.to_owned(), updated);
    drop(accounts);
    if revoke_session {
        state.sessions.write().await.remove(id);
        kick_client(&state, id).await;
        return Json(json!({"ok":true,"player":out,"sessionRevoked":true})).into_response();
    }
    push_account(&state, id).await;
    Json(json!({"ok":true,"player":out})).into_response()
}
async fn kick_client(state: &AppState, id: &str) {
    let clients = state.clients.read().await;
    if let Some(c) = clients.get(id) {
        c.send(
            axum::response::sse::Event::default()
                .data(json!({"type":"noroom","reason":"auth"}).to_string()),
        );
    }
}
pub(super) fn admin_view(a: &Account, online: bool) -> Value {
    let mut view = a.public(online);
    if let Some(fields) = view.as_object_mut() {
        fields.insert(
            "kind".into(),
            json!(if password::is_temporary(&a.id) {
                "temp"
            } else if password::is_discord(&a.id) {
                "discord"
            } else {
                "legacy"
            }),
        );
        fields.insert("hasPassword".into(), json!(a.password_hash.is_some()));
        fields.insert("createdAt".into(), json!(a.created_at));
    }
    view
}
