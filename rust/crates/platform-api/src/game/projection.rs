use super::{card_json, Account, Room, SNAPSHOT_LOG_LIMIT};
use durak_engine::{Mode, Suit, MAX_PLAYERS, MAX_TABLE_CARDS, MIN_PLAYERS};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

pub(super) fn room_summary(
    r: &Room,
    accounts: &HashMap<String, Account>,
    connected: &HashSet<String>,
) -> Value {
    json!({"id":r.id,"name":r.name,"mode":if r.mode==Mode::Transfer{"transfer"}else{"classic"},"deckSize":r.deck_size.card_count(),"bet":r.bet,"status":status(r.state.status),"host":accounts.get(&r.host).map(|a|a.name.clone()).unwrap_or_default(),"players":r.state.players.iter().filter(|p| !p.left && connected.contains(&p.id)).map(|p|json!({"id":p.id,"name":accounts.get(&p.id).map(|a|a.name.clone()).unwrap_or_else(||p.id.clone()),"cards":p.hand.len(),"connected":true})).collect::<Vec<_>>(),"seats":MAX_PLAYERS})
}
pub(super) fn status(s: durak_engine::Status) -> &'static str {
    match s {
        durak_engine::Status::Waiting => "waiting",
        durak_engine::Status::Playing => "playing",
        durak_engine::Status::Finished => "finished",
    }
}
pub(super) fn snapshot(
    r: &Room,
    id: &str,
    accounts: &std::collections::HashMap<String, Account>,
    connected: &std::collections::HashSet<String>,
) -> Value {
    let game = &r.state;
    let me = game.players.iter().find(|p| p.id == id);
    let name = |id: Option<&str>| {
        id.and_then(|id| game.players.iter().find(|p| p.id == id))
            .map(|p| {
                accounts
                    .get(&p.id)
                    .map(|a| a.name.clone())
                    .unwrap_or_else(|| p.id.clone())
            })
            .unwrap_or_default()
    };
    let role = |id: &str| {
        if game.attacker_id.as_deref() == Some(id) {
            "attacker"
        } else if game.defender_id.as_deref() == Some(id) {
            "defender"
        } else {
            "idle"
        }
    };
    let hand: Vec<Value> = me
        .map(|p| p.hand.iter().copied().map(card_json).collect())
        .unwrap_or_default();
    let playing = game.status == durak_engine::Status::Playing;
    let active = me.is_some_and(|p| !p.finished && !p.left);
    let eligible = active && me.is_some_and(|p| !p.hand.is_empty());
    let defending = game.defender_id.as_deref() == Some(id) && me.is_some();
    let unbeaten = game
        .table
        .iter()
        .filter(|row| row.defense.is_none())
        .count();
    let defender = game
        .defender_id
        .as_ref()
        .and_then(|id| game.players.iter().find(|p| &p.id == id));
    let matches_rank = me.is_some_and(|p| {
        p.hand.iter().any(|c| {
            game.table.iter().any(|row| {
                row.attack.rank == c.rank || row.defense.is_some_and(|d| d.rank == c.rank)
            })
        })
    });
    let turn = game.turn_id.as_deref() == Some(id);
    let already_passed = game.passed_attackers.iter().any(|player| player == id);
    let can_attack = eligible
        && playing
        && !defending
        && !already_passed
        && (turn
            || (!game.table.is_empty()
                && !game.take_pending
                && game.defender_id.as_deref() != Some(id)
                && game.turn_id.as_deref() == game.defender_id.as_deref()))
        && defender.is_some_and(|p| p.hand.len() > unbeaten)
        && game.table.len() < MAX_TABLE_CARDS
        && (game.table.is_empty() || matches_rank);
    let can_defend = me.is_some() && playing && defending && !game.take_pending && unbeaten > 0;
    let can_take = can_defend && !game.table.is_empty();
    let can_pass = active
        && playing
        && !defending
        && !already_passed
        && !game.table.is_empty()
        && (game.take_pending || (turn && unbeaten == 0));
    let next = me
        .and_then(|p| game.players.iter().position(|x| x.id == p.id))
        .and_then(|i| {
            (1..=game.players.len())
                .map(|n| &game.players[(i + n) % game.players.len()])
                .find(|p| !p.finished && !p.left)
        });
    let can_transfer = eligible
        && playing
        && game.mode == Mode::Transfer
        && defending
        && !game.take_pending
        && !game.table.is_empty()
        && game.table.len() < MAX_TABLE_CARDS
        && unbeaten == game.table.len()
        && game
            .table
            .iter()
            .all(|row| row.attack.rank == game.table[0].attack.rank)
        && next.is_some_and(|p| p.id != id && p.hand.len() > game.table.len())
        && me.is_some_and(|p| p.hand.iter().any(|c| c.rank == game.table[0].attack.rank));
    let can_play = can_attack || can_defend || can_transfer || can_pass;
    let can_start = me.is_some_and(|p| !p.left)
        && r.host == id
        && !playing
        && game
            .players
            .iter()
            .filter(|p| !p.left && connected.contains(&p.id))
            .count()
            >= MIN_PLAYERS;
    let finished = game.finished.as_ref().map(|f| json!({"loser":f.loser_id.as_deref().map(|id| name(Some(id))).unwrap_or_else(|| "—".into()),"loserId":f.loser_id,"draw":f.draw}));
    json!({
        "room":{"id":r.id,"name":r.name,"mode":if r.mode==Mode::Transfer{"transfer"}else{"classic"},"deckSize":r.deck_size.card_count(),"bet":r.bet,"status":status(game.status),"hostId":r.host},
        "players":game.players.iter().enumerate().map(|(i,p)|json!({"id":p.id,"name":name(Some(&p.id)),"seat":i,"cards":p.hand.len(),"connected":connected.contains(&p.id),"finished":p.finished,"left":p.left,"role":role(&p.id)})).collect::<Vec<_>>(),
        "you":me.map(|p|json!({"id":p.id,"name":name(Some(&p.id)),"finished":p.finished,"left":p.left,"role":role(&p.id),"turn":can_play,"canPlay":can_play,"hand":hand})),
        "hand":hand,"trump":game.trump.map(|s|match s {Suit::Spades=>"♠",Suit::Hearts=>"♥",Suit::Diamonds=>"♦",Suit::Clubs=>"♣"}),"trumpCard":game.trump_card.map(card_json),"deck":game.deck.len(),
        "table":game.table.iter().map(|row|json!({"attack":card_json(row.attack),"defense":row.defense.map(card_json)})).collect::<Vec<_>>(),
        "attackerId":game.attacker_id,"defenderId":game.defender_id,"turnId":game.turn_id,"attackerName":name(game.attacker_id.as_deref()),"defenderName":name(game.defender_id.as_deref()),"turnName":name(game.turn_id.as_deref()),
        "message":r.message,"canAttack":can_attack,"canDefend":can_defend,"canTransfer":can_transfer,"canPass":can_pass,"canTake":can_take,"takePending":game.take_pending,"canStart":can_start,"payout":r.payouts.get(id).copied().unwrap_or(0),"finished":finished,"seq":r.seq,"log":r.events.iter().rev().take(SNAPSHOT_LOG_LIMIT).rev().collect::<Vec<_>>()
    })
}
