use super::{error, game, oauth, password, AppState};
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

pub(super) async fn auth_providers() -> Json<serde_json::Value> {
    Json(json!({"providers": ["temporary"]}))
}

pub(super) async fn temp_login(
    State(state): State<AppState>,
    Json(body): Json<serde_json::Value>,
) -> Response {
    let name = body
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    let password = body
        .get("password")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if name.is_empty() || name.chars().count() > 20 || password.len() < 4 || password.len() > 128 {
        return error(
            StatusCode::BAD_REQUEST,
            "Укажите имя и пароль от 4 до 128 символов",
        );
    }
    let repository = match &state.repository {
        Some(repository) => repository,
        None => return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно"),
    };
    let matches = |account: &game::Account| {
        account
            .password_hash
            .as_deref()
            .is_some_and(|stored| password::verify(password, stored))
    };
    let mut account = {
        let accounts = state.accounts.read().await;
        accounts
            .values()
            .find(|account| account.name.to_lowercase() == name.to_lowercase() && matches(account))
            .cloned()
            .map(|account| (account, false))
    };
    if account.is_none() {
        let stored = repository.lock().await.list_accounts().unwrap_or_default();
        account = stored
            .into_iter()
            .find(|account| account.name.to_lowercase() == name.to_lowercase() && matches(account))
            .map(|account| {
                let upgrade =
                    password::needs_upgrade(account.password_hash.as_deref().unwrap_or_default());
                (account, upgrade)
            });
    }
    let Some((stored, upgrade)) = account else {
        password::burn(password);
        return error(StatusCode::FORBIDDEN, "Неверное имя или пароль");
    };
    let password_hash = if upgrade {
        password::hash(password)
    } else {
        stored.password_hash.clone().unwrap_or_default()
    };
    let account = game::Account {
        password_hash: Some(password_hash.clone()),
        ..stored
    };
    let token = match oauth::random() {
        Ok(token) => oauth::digest(&token),
        Err(response) => return response.into_response(),
    };
    {
        let mut repository = repository.lock().await;
        if repository
            .finish_temp_login(&account.id, &oauth::digest(&token), &password_hash)
            .is_err()
        {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
        if repository.save_account(&account).is_err() {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
    }
    state
        .accounts
        .write()
        .await
        .insert(account.id.clone(), account.clone());
    state
        .sessions
        .write()
        .await
        .insert(account.id.clone(), token.clone());
    state.clients.write().await.remove(&account.id);
    Json(json!({"clientId": account.id, "token": token})).into_response()
}
