use super::{now, snapshot, status, Account, AppState, Room, EVENT_LIMIT};
use durak_engine::Mode;
use serde_json::{json, Value};
use std::collections::HashMap;

fn record_room(
    r: &mut Room,
    accounts: &HashMap<String, Account>,
    actor: &str,
    action: &str,
    data: &Value,
) {
    r.seq += 1;
    let ev = json!({"seq":r.seq,"at":now(),"action":action,"actorId":if actor.is_empty(){None}else{Some(actor)},"actorName":accounts.get(actor).map(|a|a.name.as_str()).unwrap_or(""),"cardId":data.get("cardId"),"targetCardId":data.get("targetCardId"),"status":status(r.state.status),"turnId":r.state.turn_id,"attackerId":r.state.attacker_id,"defenderId":r.state.defender_id,"message":r.message});
    r.events.push(ev);
    if r.events.len() > EVENT_LIMIT {
        r.events.remove(0);
    }
}

fn durable_account(a: &Account) -> super::super::storage::DurableAccount {
    super::super::storage::DurableAccount {
        id: a.id.clone(),
        name: a.name.clone(),
        balance: a.balance,
        games: a.games,
        wins: a.wins,
        losses: a.losses,
        avatar_url: a.avatar_url.clone(),
        password_hash: a.password_hash.clone(),
        created_at: a.created_at,
    }
}

pub(super) struct CommitMeta<'a> {
    pub(super) actor: &'a str,
    pub(super) action: &'a str,
    pub(super) data: &'a Value,
}

pub(super) async fn commit_room(
    state: &AppState,
    r: &mut Room,
    before: &HashMap<String, Account>,
    after: &HashMap<String, Account>,
    meta: CommitMeta<'_>,
    initial: bool,
) -> Result<(), ()> {
    let was_finished = r
        .events
        .last()
        .and_then(|event| event.get("status"))
        .and_then(Value::as_str)
        == Some("finished");
    record_room(r, after, meta.actor, meta.action, meta.data);
    if let Some(repository) = &state.repository {
        let seq = i64::try_from(r.seq)
            .map_err(|_| ())?
            .checked_sub(1)
            .ok_or(())?;
        if before.keys().any(|id| !after.contains_key(id)) {
            return Err(());
        }
        let mut changes: Vec<_> = after
            .iter()
            .filter_map(|(id, new)| {
                let old = before.get(id);
                if old == Some(new) {
                    None
                } else {
                    Some(super::super::storage::AccountChange {
                        before: old.map(durable_account),
                        after: durable_account(new),
                    })
                }
            })
            .collect();
        changes.sort_by(|a, b| a.after.id.cmp(&b.after.id));
        let transition = super::super::storage::DurableTransition {
            game_id: r.id.clone(),
            expected_seq: if initial { None } else { Some(seq - 1) },
            seq,
            state_json: serde_json::to_string(r).map_err(|_| ())?,
            accounts: changes,
        };
        let history = if r.state.status == durak_engine::Status::Finished && !was_finished {
            let finished = r.state.finished.as_ref();
            let loser = match finished {
                Some(f) if f.draw => "Ничья".to_owned(),
                Some(f) => f
                    .loser_id
                    .as_deref()
                    .and_then(|id| after.get(id).map(|a| a.name.clone()))
                    .unwrap_or_else(|| "Неизвестный игрок".into()),
                None => "Ничья".to_owned(),
            };
            Some(super::super::storage::FinishedGameHistory {
                room: r.name.clone(),
                mode: if r.mode == Mode::Transfer {
                    "transfer".into()
                } else {
                    "classic".into()
                },
                at: now() as i64,
                loser,
                bet: r.bet,
            })
        } else {
            None
        };
        repository
            .lock()
            .await
            .commit_transition_with_history(&transition, history.as_ref())
            .map_err(|_| ())?;
    }
    Ok(())
}

pub(super) async fn publish_room(state: &AppState, r: &Room) {
    let accounts = state.accounts.read().await;
    let clients = state.clients.read().await;
    let connected = clients.keys().cloned().collect();
    for p in &r.state.players {
        if !p.left {
            if let Some(c) = clients.get(&p.id) {
                c.send(
                    axum::response::sse::Event::default().data(
                        json!({"type":"game","state":snapshot(r,&p.id,&accounts,&connected)})
                            .to_string(),
                    ),
                );
            }
        }
    }
    drop(clients);
    drop(accounts);
    if let Some(ev) = r.events.last() {
        state.event_hub.publish(r.id.clone(), ev.clone()).await;
    }
}

pub(super) async fn push_snapshot(state: &AppState, r: &Room, id: &str) {
    let accounts = state.accounts.read().await;
    let clients = state.clients.read().await;
    if let Some(c) = clients.get(id) {
        let connected = clients.keys().cloned().collect();
        c.send(axum::response::sse::Event::default().data(
            json!({"type":"game","state":snapshot(r, id, &accounts, &connected)}).to_string(),
        ));
    }
}

pub(super) async fn broadcast(
    state: &AppState,
    r: &mut Room,
    actor: &str,
    action: &str,
    data: &Value,
) {
    let accounts = state.accounts.read().await;
    let mut next = r.clone();
    if commit_room(
        state,
        &mut next,
        &accounts,
        &accounts,
        CommitMeta {
            actor,
            action,
            data,
        },
        false,
    )
    .await
    .is_err()
    {
        return;
    }
    *r = next;
    drop(accounts);
    publish_room(state, r).await;
}
