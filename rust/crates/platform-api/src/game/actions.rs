use super::cards::{card, card_label};
use super::{
    auth, broadcast_lobby, commit_room, error, publish_room, push_account, settle, AppState,
    CommitMeta, SecureRandom,
};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use durak_engine::{Action, Command, DurakState};
use serde_json::{json, Value};
use std::collections::HashSet;

fn play(
    game: DurakState,
    id: String,
    action: Action,
) -> Result<DurakState, durak_engine::RuleError> {
    let command = |player: &str, action: Action| Command {
        player_id: player.to_owned(),
        action,
    };
    match action {
        Action::Attack(_) => {
            if !game.table.is_empty()
                && !game.take_pending
                && game.defender_id.as_deref() != Some(id.as_str())
                && game.turn_id.as_deref() == game.defender_id.as_deref()
            {
                let mut next = game.clone();
                next.passed_attackers.retain(|p| p != &id);
                next.turn_id = Some(id.clone());
                return next.apply(command(&id, action)).map(|t| t.state);
            }
        }
        Action::Pass
            if game.take_pending
                && game.turn_id.as_deref() != Some(id.as_str())
                && game.defender_id.as_deref() != Some(id.as_str()) =>
        {
            let mut probe = game.clone();
            probe.passed_attackers.retain(|p| p != &id);
            probe.turn_id = Some(id.clone());
            if let Ok(passed) = probe.apply(command(&id, Action::Pass)) {
                return Ok(passed.state);
            }
        }
        _ => {}
    }
    game.apply(command(&id, action)).map(|t| t.state)
}

pub async fn action(
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
    let mut rooms = state.rooms.write().await;
    let Some(r) = rooms
        .values_mut()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    else {
        return error(StatusCode::BAD_REQUEST, "Вы не находитесь в лобби");
    };
    let mut next_room = r.clone();
    let mut accounts_guard = state.accounts.write().await;
    let mut next_accounts = accounts_guard.clone();
    let a = body.get("action").and_then(Value::as_str).unwrap_or("");
    if a == "start" {
        if next_room.host != id {
            return error(StatusCode::BAD_REQUEST, "Игру начинает создатель лобби");
        }
        if next_room.state.status == durak_engine::Status::Playing {
            return error(StatusCode::BAD_REQUEST, "Партия уже идёт");
        }
        let connected: HashSet<_> = state.clients.read().await.keys().cloned().collect();
        let ready: Vec<_> = next_room
            .state
            .players
            .iter()
            .filter(|p| connected.contains(&p.id) && !p.left)
            .map(|p| p.id.clone())
            .collect();
        if ready.len() < 2 {
            return error(StatusCode::BAD_REQUEST, "Нужно минимум два игрока");
        }
        if ready.iter().any(|id| {
            next_accounts
                .get(id)
                .is_none_or(|a| a.balance < next_room.bet)
        }) {
            return error(StatusCode::BAD_REQUEST, "Недостаточно фишек");
        }
        let mut game = DurakState::new(ready.clone(), next_room.mode, next_room.deck_size);
        if game.start(&mut SecureRandom).is_err() {
            return error(StatusCode::BAD_REQUEST, "Не удалось начать игру");
        }
        for id in &ready {
            next_accounts.get_mut(id).unwrap().balance -= next_room.bet;
        }
        next_room.pot = next_room.bet * ready.len() as i64;
        next_room.payouts.clear();
        next_room.state = game;
        next_room.message = "Партия началась".into();
    } else {
        let c = body.get("cardId").and_then(card);
        let action = match a {
            "attack" => c.map(Action::Attack),
            "defend" => c.map(|x| Action::Defend {
                card: x,
                target: body.get("targetCardId").and_then(card),
            }),
            "transfer" => c.map(Action::Transfer),
            "take" => Some(Action::Take),
            "pass" => Some(Action::Pass),
            _ => None,
        };
        let result = action
            .ok_or(durak_engine::RuleError::CardNotInHand)
            .and_then(|x| {
                let throw_in = matches!(x, Action::Attack(_)) && !next_room.state.table.is_empty();
                if throw_in && next_room.state.take_pending {
                    next_room.state.passed_attackers.retain(|p| {
                        p != &id && p != &next_room.state.turn_id.clone().unwrap_or_default()
                    });
                }
                play(next_room.state.clone(), id.clone(), x)
            });
        let next = match result {
            Ok(next) => next,
            Err(_) => return error(StatusCode::BAD_REQUEST, "Недопустимое действие"),
        };
        let actor_name = next_accounts
            .get(&id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| id.clone());
        let card_text = |key: &str| {
            body.get(key)
                .and_then(card)
                .map(card_label)
                .unwrap_or_default()
        };
        next_room.state = next;
        let next_name = next_room
            .state
            .turn_id
            .as_ref()
            .and_then(|turn| next_accounts.get(turn))
            .map(|a| a.name.clone())
            .unwrap_or_default();
        next_room.message = match a {
            "attack" if next_room.state.table.len() > 1 => {
                format!("{actor_name} подкидывает {}", card_text("cardId"))
            }
            "attack" => format!("{actor_name} ходит {}", card_text("cardId")),
            "defend" => format!("{actor_name} отбивает {}", card_text("cardId")),
            "transfer" => format!("{actor_name} переводит ход на {next_name}"),
            "take" => format!("{actor_name} забирает карты со стола"),
            "pass" if next_name.is_empty() => "Бито".to_owned(),
            "pass" => format!("Бито. Ход у {next_name}"),
            _ => next_room.message.clone(),
        };
        if next_room.state.status == durak_engine::Status::Finished {
            let loser = next_room
                .state
                .finished
                .as_ref()
                .and_then(|f| f.loser_id.clone());
            let loser_name = loser
                .as_ref()
                .and_then(|loser| next_accounts.get(loser))
                .map(|a| a.name.clone());
            settle(&mut next_accounts, &mut next_room, loser);
            next_room.message = match loser_name {
                Some(name) => format!("{name} — дурак, партия окончена"),
                None => "Ничья. Ставки возвращены".to_owned(),
            };
        }
    }
    if commit_room(
        &state,
        &mut next_room,
        &accounts_guard,
        &next_accounts,
        CommitMeta {
            actor: &id,
            action: a,
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
    *accounts_guard = next_accounts;
    drop(accounts_guard);
    publish_room(&state, r).await;
    for p in &r.state.players {
        push_account(&state, &p.id).await;
    }
    broadcast_lobby(&state, &rooms).await;
    (StatusCode::OK, Json(json!({"ok":true}))).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use durak_engine::{Card, DeckSize, Mode, Rank, Status, Suit, TableRow};

    #[test]
    fn out_of_turn_pass_preserves_remaining_thrower_and_completes_take() {
        let mut game = DurakState::new(
            vec!["a".into(), "b".into(), "c".into()],
            Mode::Classic,
            DeckSize::ThirtySix,
        );
        game.start(&mut durak_engine::Lcg(7)).unwrap();
        game.status = Status::Playing;
        game.attacker_id = Some("a".into());
        game.defender_id = Some("b".into());
        game.turn_id = Some("a".into());
        game.take_pending = true;
        game.table = vec![TableRow {
            attack: Card::new(Rank::Six, Suit::Hearts),
            defense: None,
        }];
        let game = play(game, "c".into(), Action::Pass).unwrap();
        assert_eq!(game.turn_id.as_deref(), Some("a"));
        assert_eq!(game.passed_attackers, vec!["c"]);
        let game = play(game, "a".into(), Action::Pass).unwrap();
        assert!(!game.take_pending);
        assert!(game.table.is_empty());
    }
}
