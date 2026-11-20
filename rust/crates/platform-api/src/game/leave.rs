use super::{
    auth, broadcast_lobby, commit_room, error, publish_room, push_account, settle, AppState,
    CommitMeta, Room,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::{json, Value};
use std::collections::HashMap;

pub(super) async fn leave_inner(
    state: &AppState,
    rooms: &mut HashMap<String, Room>,
    rid: &str,
    id: &str,
    cancel_duel: bool,
) -> Result<(), ()> {
    let Some(current) = rooms.get_mut(rid) else {
        return Ok(());
    };
    let mut room = current.clone();
    let Some(index) = room
        .state
        .players
        .iter()
        .position(|p| p.id == id && !p.left)
    else {
        return Ok(());
    };
    let mut accounts = state.accounts.write().await;
    let original_accounts = accounts.clone();
    let mut next_accounts = accounts.clone();
    if room.state.status == durak_engine::Status::Playing {
        let active = room.state.players.iter().filter(|p| !p.left).count();
        room.state.players[index].left = true;
        if room.host == id {
            if let Some(p) = room.state.players.iter().find(|p| !p.left) {
                room.host = p.id.clone();
            }
        }
        if cancel_duel && active <= 2 {
            for p in &room.state.players {
                if let Some(account) = next_accounts.get_mut(&p.id) {
                    account.balance += room.bet;
                }
            }
            room.state.status = durak_engine::Status::Waiting;
            room.state.players.remove(index);
            room.state.finished = None;
            room.pot = 0;
            room.message = "Партия отменена: игрок удалён администратором".into();
        } else if !room.state.players[index].finished {
            room.state.status = durak_engine::Status::Finished;
            room.state.finished = Some(durak_engine::Finished {
                loser_id: Some(id.to_owned()),
                draw: false,
            });
            room.message = "Игрок покинул партию, игра окончена".into();
            settle(&mut next_accounts, &mut room, Some(id.to_owned()));
        }
    } else {
        room.state.players.remove(index);
        if room.host == id {
            if let Some(p) = room.state.players.first() {
                room.host = p.id.clone();
            }
        }
    }
    let leave_body = json!({});
    commit_room(
        state,
        &mut room,
        &original_accounts,
        &next_accounts,
        CommitMeta {
            actor: id,
            action: "leave",
            data: &leave_body,
        },
        false,
    )
    .await?;
    *accounts = next_accounts;
    drop(accounts);
    if room.state.players.is_empty() {
        rooms.remove(rid);
    } else {
        *current = room;
        publish_room(state, current).await;
        for p in &current.state.players {
            push_account(state, &p.id).await;
        }
    }
    push_account(state, id).await;
    Ok(())
}
pub async fn leave(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return *e,
    };
    let mut rooms = state.rooms.write().await;
    let rid = rooms
        .iter()
        .find(|(_, r)| r.state.players.iter().any(|p| p.id == id && !p.left))
        .map(|(id, _)| id.clone());
    if let Some(rid) = rid {
        if leave_inner(&state, &mut rooms, &rid, &id, false)
            .await
            .is_err()
        {
            return error(StatusCode::SERVICE_UNAVAILABLE, "Хранилище недоступно");
        }
    }
    broadcast_lobby(&state, &rooms).await;
    if let Some(c) = state.clients.read().await.get(&id) {
        c.send(axum::response::sse::Event::default().data(json!({"type":"noroom"}).to_string()));
    }
    (StatusCode::OK, Json(json!({"ok":true}))).into_response()
}
