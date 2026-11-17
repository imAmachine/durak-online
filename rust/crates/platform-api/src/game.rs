use super::{error, is_client_id, is_token, AppState};
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use durak_engine::{Action, Card, Command, DeckSize, DurakState, Mode, Rank, Suit};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
struct SecureRandom;
impl durak_engine::RandomSource for SecureRandom {
    fn next_usize(&mut self, upper: usize) -> usize {
        let zone = u64::MAX - u64::MAX % upper as u64;
        loop {
            let mut bytes = [0u8; 8];
            getrandom::getrandom(&mut bytes).expect("OS random source unavailable");
            let n = u64::from_ne_bytes(bytes);
            if n < zone {
                return (n % upper as u64) as usize;
            }
        }
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Clone)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub balance: i64,
    pub games: i64,
    pub wins: i64,
    pub losses: i64,
}
impl Account {
    pub fn new(id: &str, name: &str) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            balance: 1000,
            games: 0,
            wins: 0,
            losses: 0,
        }
    }
    pub fn public(&self, online: bool) -> Value {
        json!({"id":self.id,"name":self.name,"balance":self.balance,"games":self.games,"wins":self.wins,"losses":self.losses,"online":online})
    }
}

pub struct Room {
    pub id: String,
    pub name: String,
    pub mode: Mode,
    pub deck_size: DeckSize,
    pub bet: i64,
    pub host: String,
    pub state: DurakState,
    pub seq: u64,
    pub events: Vec<Value>,
    pub pot: i64,
    pub payouts: HashMap<String, i64>,
    pub message: String,
}

async fn auth(state: &AppState, body: &Value) -> Result<String, axum::response::Response> {
    let id = body.get("clientId").and_then(Value::as_str).unwrap_or("");
    let token = body.get("token").and_then(Value::as_str).unwrap_or("");
    if !is_client_id(id) || !is_token(token) {
        return Err(error(StatusCode::FORBIDDEN, "Неверный токен сессии"));
    }
    if state.sessions.read().await.get(id).map(String::as_str) == Some(token) {
        Ok(id.to_owned())
    } else {
        Err(error(StatusCode::FORBIDDEN, "Неверный токен сессии"))
    }
}
fn card(v: &Value) -> Option<Card> {
    let id = v.as_str().or_else(|| v.get("id").and_then(Value::as_str))?;
    let (pos, _) = id.char_indices().last()?;
    let (rank, suit) = id.split_at(pos);
    Some(Card::new(
        match rank {
            "2" => Rank::Two,
            "3" => Rank::Three,
            "4" => Rank::Four,
            "5" => Rank::Five,
            "6" => Rank::Six,
            "7" => Rank::Seven,
            "8" => Rank::Eight,
            "9" => Rank::Nine,
            "10" => Rank::Ten,
            "В" | "J" => Rank::Jack,
            "Д" | "Q" => Rank::Queen,
            "К" | "K" => Rank::King,
            "Т" | "A" => Rank::Ace,
            _ => return None,
        },
        match suit {
            "♠" => Suit::Spades,
            "♥" => Suit::Hearts,
            "♦" => Suit::Diamonds,
            "♣" => Suit::Clubs,
            _ => return None,
        },
    ))
}
fn card_json(c: Card) -> Value {
    json!({"id":format!("{}{}",match c.rank {Rank::Two=>"2",Rank::Three=>"3",Rank::Four=>"4",Rank::Five=>"5",Rank::Six=>"6",Rank::Seven=>"7",Rank::Eight=>"8",Rank::Nine=>"9",Rank::Ten=>"10",Rank::Jack=>"В",Rank::Queen=>"Д",Rank::King=>"К",Rank::Ace=>"Т"},match c.suit {Suit::Spades=>"♠",Suit::Hearts=>"♥",Suit::Diamonds=>"♦",Suit::Clubs=>"♣"}),"rank":match c.rank {Rank::Two=>"2",Rank::Three=>"3",Rank::Four=>"4",Rank::Five=>"5",Rank::Six=>"6",Rank::Seven=>"7",Rank::Eight=>"8",Rank::Nine=>"9",Rank::Ten=>"10",Rank::Jack=>"В",Rank::Queen=>"Д",Rank::King=>"К",Rank::Ace=>"Т"},"suit":match c.suit {Suit::Spades=>"♠",Suit::Hearts=>"♥",Suit::Diamonds=>"♦",Suit::Clubs=>"♣"}})
}
fn room_summary(r: &Room, accounts: &std::collections::HashMap<String, Account>) -> Value {
    json!({"id":r.id,"name":r.name,"mode":if r.mode==Mode::Transfer{"transfer"}else{"classic"},"deckSize":r.deck_size.card_count(),"bet":r.bet,"status":status(r.state.status),"host":accounts.get(&r.host).map(|a|a.name.clone()).unwrap_or_default(),"players":r.state.players.iter().map(|p|json!({"id":p.id,"name":accounts.get(&p.id).map(|a|a.name.clone()).unwrap_or_else(||p.id.clone()),"cards":p.hand.len(),"connected":true})).collect::<Vec<_>>(),"seats":6})
}
fn status(s: durak_engine::Status) -> &'static str {
    match s {
        durak_engine::Status::Waiting => "waiting",
        durak_engine::Status::Playing => "playing",
        durak_engine::Status::Finished => "finished",
    }
}
fn snapshot(
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
    let eligible = me.is_some_and(|p| !p.finished && !p.left && !p.hand.is_empty());
    let attacking = game.attacker_id.as_deref() == Some(id) && me.is_some();
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
    let can_attack = eligible
        && playing
        && !defending
        && (attacking || !game.table.is_empty())
        && defender.is_some_and(|p| p.hand.len() > unbeaten)
        && game.table.len() < 6
        && (game.table.is_empty() || matches_rank);
    let can_defend = me.is_some() && playing && defending && !game.take_pending && unbeaten > 0;
    let can_take = can_defend && !game.table.is_empty();
    let can_pass = me.is_some()
        && playing
        && attacking
        && !game.table.is_empty()
        && (game.take_pending || unbeaten == 0);
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
        && game.table.len() < 6
        && unbeaten == game.table.len()
        && game
            .table
            .iter()
            .all(|row| row.attack.rank == game.table[0].attack.rank)
        && next.is_some_and(|p| p.id != id && p.hand.len() > game.table.len())
        && me.is_some_and(|p| p.hand.iter().any(|c| c.rank == game.table[0].attack.rank));
    let can_play = can_attack || can_defend;
    let can_start = me.is_some_and(|p| !p.left)
        && r.host == id
        && !playing
        && game
            .players
            .iter()
            .filter(|p| !p.left && connected.contains(&p.id))
            .count()
            >= 2;
    let finished = game.finished.as_ref().map(|f| json!({"loser":f.loser_id.as_deref().map(|id| name(Some(id))).unwrap_or_else(|| "—".into()),"loserId":f.loser_id,"draw":f.draw}));
    json!({
        "room":{"id":r.id,"name":r.name,"mode":if r.mode==Mode::Transfer{"transfer"}else{"classic"},"deckSize":r.deck_size.card_count(),"bet":r.bet,"status":status(game.status),"hostId":r.host},
        "players":game.players.iter().enumerate().map(|(i,p)|json!({"id":p.id,"name":name(Some(&p.id)),"seat":i,"cards":p.hand.len(),"connected":connected.contains(&p.id),"finished":p.finished,"left":p.left,"role":role(&p.id)})).collect::<Vec<_>>(),
        "you":me.map(|p|json!({"id":p.id,"name":name(Some(&p.id)),"finished":p.finished,"left":p.left,"role":role(&p.id),"turn":can_play,"canPlay":can_play,"hand":hand})),
        "hand":hand,"trump":game.trump.map(|s|match s {Suit::Spades=>"♠",Suit::Hearts=>"♥",Suit::Diamonds=>"♦",Suit::Clubs=>"♣"}),"trumpCard":game.trump_card.map(card_json),"deck":game.deck.len(),
        "table":game.table.iter().map(|row|json!({"attack":card_json(row.attack),"defense":row.defense.map(card_json)})).collect::<Vec<_>>(),
        "attackerId":game.attacker_id,"defenderId":game.defender_id,"turnId":game.turn_id,"attackerName":name(game.attacker_id.as_deref()),"defenderName":name(game.defender_id.as_deref()),"turnName":name(game.turn_id.as_deref()),
        "message":r.message,"canAttack":can_attack,"canDefend":can_defend,"canTransfer":can_transfer,"canPass":can_pass,"canTake":can_take,"takePending":game.take_pending,"canStart":can_start,"payout":r.payouts.get(id).copied().unwrap_or(0),"finished":finished,"seq":r.seq,"log":r.events.iter().rev().take(8).rev().collect::<Vec<_>>()
    })
}
async fn push_account(state: &AppState, id: &str) {
    let clients = state.clients.read().await;
    if let Some(c) = clients.get(id) {
        if let Some(a) = state.accounts.read().await.get(id) {
            let _ = c.sender.try_send(Ok(axum::response::sse::Event::default()
                .data(json!({"type":"account","account":a.public(true)}).to_string())));
        }
    }
}
async fn broadcast(state: &AppState, r: &mut Room, actor: &str, action: &str, data: &Value) {
    r.seq += 1;
    let accounts = state.accounts.read().await;
    let ev = json!({"seq":r.seq,"at":now(),"action":action,"actorId":if actor.is_empty(){None}else{Some(actor)},"actorName":accounts.get(actor).map(|a|a.name.as_str()).unwrap_or(""),"cardId":data.get("cardId"),"targetCardId":data.get("targetCardId"),"status":status(r.state.status),"turnId":r.state.turn_id,"attackerId":r.state.attacker_id,"defenderId":r.state.defender_id,"message":r.message});
    r.events.push(ev.clone());
    if r.events.len() > 40 {
        r.events.remove(0);
    }
    let clients = state.clients.read().await;
    let connected = clients.keys().cloned().collect();
    for p in &r.state.players {
        if !p.left {
            if let Some(c) = clients.get(&p.id) {
                let _ = c
                    .sender
                    .try_send(Ok(axum::response::sse::Event::default().data(
                        json!({"type":"game","state":snapshot(r,&p.id,&accounts,&connected)})
                            .to_string(),
                    )));
            }
        }
    }
    drop(clients);
    drop(accounts);
    state.event_hub.publish(r.id.clone(), ev).await;
}
async fn settle(state: &AppState, r: &mut Room, loser: Option<String>) {
    let mut accounts = state.accounts.write().await;
    let winners: Vec<_> = r
        .state
        .players
        .iter()
        .filter(|p| Some(&p.id) != loser.as_ref())
        .map(|p| p.id.clone())
        .collect();
    r.payouts.clear();
    for p in &r.state.players {
        if let Some(a) = accounts.get_mut(&p.id) {
            a.games += 1;
            if loser.as_ref() == Some(&p.id) {
                a.losses += 1;
            } else if loser.is_some() {
                a.wins += 1;
            }
        }
    }
    if !winners.is_empty() {
        let share = r.pot / winners.len() as i64;
        let rem = r.pot % winners.len() as i64;
        for (i, id) in winners.iter().enumerate() {
            let amount = share + i64::from((i as i64) < rem);
            if let Some(a) = accounts.get_mut(id) {
                a.balance += amount;
            }
            r.payouts.insert(id.clone(), amount - r.bet);
        }
    }
    if let Some(id) = loser {
        r.payouts.insert(id, -r.bet);
    }
    r.pot = 0;
    drop(accounts);
    for p in &r.state.players {
        push_account(state, &p.id).await;
    }
}

pub async fn create(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let clients = state.clients.read().await;
    let Some(client) = clients.get(&id) else {
        return error(StatusCode::BAD_REQUEST, "Нет соединения с сервером");
    };
    let name = body
        .get("playerName")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(&client.name)
        .chars()
        .take(20)
        .collect::<String>();
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
            break code;
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
    let bet = if [0, 50, 100, 250, 500].contains(&requested) {
        requested
    } else {
        0
    };
    let mut accounts = state.accounts.write().await;
    let account = accounts
        .entry(id.clone())
        .or_insert_with(|| Account::new(&id, &name));
    if account.balance < bet {
        return error(StatusCode::BAD_REQUEST, "Недостаточно фишек");
    };
    account.name = name;
    let account = account.public(true);
    let mut r = Room {
        id: rid.clone(),
        name: body
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Новая партия")
            .chars()
            .take(40)
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
    let summary = room_summary(&r, &accounts);
    drop(accounts);
    broadcast(&state, &mut r, &id, "create", &body).await;
    rooms.insert(rid.clone(), r);
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
        Err(e) => return e,
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
    let existing = target.state.players.iter().any(|p| p.id == id && !p.left);
    if target.state.status == durak_engine::Status::Playing && !existing {
        return error(StatusCode::BAD_REQUEST, "Игра уже началась");
    };
    if !existing && target.state.players.len() >= 6 {
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
    if let Some(old) = previous {
        leave_inner(&state, &mut rooms, &old, &id).await;
    }
    let r = rooms.get_mut(&rid).unwrap();
    if !existing {
        r.state.players.push(durak_engine::Player {
            id: id.clone(),
            hand: vec![],
            finished: false,
            left: false,
        });
    }
    if let Some(name) = body
        .get("playerName")
        .and_then(Value::as_str)
        .filter(|n| !n.is_empty())
    {
        if let Some(a) = state.accounts.write().await.get_mut(&id) {
            a.name = name.chars().take(20).collect();
        }
    }
    r.message = "Игроки готовы. Создатель лобби может начать игру".into();
    let accounts = state.accounts.read().await;
    let out = json!({"room":room_summary(r,&accounts),"account":accounts.get(&id).map(|a|a.public(true))});
    drop(accounts);
    broadcast(&state, r, &id, "join", &body).await;
    (StatusCode::OK, Json(out)).into_response()
}
pub async fn action(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return e,
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
    let a = body.get("action").and_then(Value::as_str).unwrap_or("");
    if a == "start" {
        if r.host != id {
            return error(StatusCode::BAD_REQUEST, "Игру начинает создатель лобби");
        };
        if r.state.status == durak_engine::Status::Playing {
            return error(StatusCode::BAD_REQUEST, "Партия уже идёт");
        };
        let connected = state
            .clients
            .read()
            .await
            .keys()
            .cloned()
            .collect::<std::collections::HashSet<_>>();
        let ready: Vec<_> = r
            .state
            .players
            .iter()
            .filter(|p| connected.contains(&p.id) && !p.left)
            .map(|p| p.id.clone())
            .collect();
        if ready.len() < 2 {
            return error(StatusCode::BAD_REQUEST, "Нужно минимум два игрока");
        };
        let mut accounts = state.accounts.write().await;
        if ready
            .iter()
            .any(|id| accounts.get(id).map_or(true, |a| a.balance < r.bet))
        {
            return error(StatusCode::BAD_REQUEST, "Недостаточно фишек");
        };
        let mut game = DurakState::new(ready.clone(), r.mode, r.deck_size);
        if game.start(&mut SecureRandom).is_err() {
            return error(StatusCode::BAD_REQUEST, "Не удалось начать игру");
        };
        for id in &ready {
            accounts.get_mut(id).unwrap().balance -= r.bet;
        }
        r.pot = r.bet * ready.len() as i64;
        r.payouts.clear();
        r.state = game;
        r.message = "Партия началась".into();
        drop(accounts);
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
                r.state
                    .apply(Command {
                        player_id: id.clone(),
                        action: x,
                    })
                    .map(|t| t.state)
            });
        let Ok(next) = result else {
            return error(StatusCode::BAD_REQUEST, "Недопустимое действие");
        };
        r.state = next;
        r.message = format!("{}: {}", id, a);
        if r.state.status == durak_engine::Status::Finished {
            let loser = r.state.finished.as_ref().and_then(|f| f.loser_id.clone());
            settle(&state, r, loser).await;
        }
    }
    broadcast(&state, r, &id, a, &body).await;
    if a == "start" {
        for p in &r.state.players {
            push_account(&state, &p.id).await;
        }
    }
    (StatusCode::OK, Json(json!({"ok":true}))).into_response()
}
async fn leave_inner(state: &AppState, rooms: &mut HashMap<String, Room>, rid: &str, id: &str) {
    let Some(r) = rooms.get_mut(rid) else { return };
    let Some(index) = r.state.players.iter().position(|p| p.id == id && !p.left) else {
        return;
    };
    if r.state.status == durak_engine::Status::Playing {
        r.state.players[index].left = true;
        if r.host == id {
            if let Some(p) = r.state.players.iter().find(|p| !p.left) {
                r.host = p.id.clone();
            }
        }
        if !r.state.players[index].finished {
            r.state.status = durak_engine::Status::Finished;
            r.state.finished = Some(durak_engine::Finished {
                loser_id: Some(id.to_owned()),
                draw: false,
            });
            r.message = "Игрок покинул партию, игра окончена".into();
            settle(state, r, Some(id.to_owned())).await;
        }
    } else {
        r.state.players.remove(index);
        if r.host == id {
            if let Some(p) = r.state.players.first() {
                r.host = p.id.clone();
            }
        }
    }
    if r.state.players.is_empty() {
        rooms.remove(rid);
        return;
    }
    broadcast(state, r, id, "leave", &json!({})).await;
}
pub async fn leave(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let mut rooms = state.rooms.write().await;
    let rid = rooms
        .iter()
        .find(|(_, r)| r.state.players.iter().any(|p| p.id == id && !p.left))
        .map(|(id, _)| id.clone());
    if let Some(rid) = rid {
        leave_inner(&state, &mut rooms, &rid, &id).await;
    }
    if let Some(c) = state.clients.read().await.get(&id) {
        let _ =
            c.sender
                .try_send(Ok(axum::response::sse::Event::default()
                    .data(json!({"type":"noroom"}).to_string())));
    }
    (StatusCode::OK, Json(json!({"ok":true}))).into_response()
}
pub async fn profile(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    let id = match auth(&state, &body).await {
        Ok(x) => x,
        Err(e) => return e,
    };
    let name = body
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if name.is_empty() {
        return error(StatusCode::BAD_REQUEST, "Имя не может быть пустым");
    };
    let mut accounts = state.accounts.write().await;
    let account = accounts
        .entry(id.clone())
        .or_insert_with(|| Account::new(&id, "Игрок"));
    account.name = name.chars().take(20).collect();
    let out = json!({"ok":true,"name":account.name,"account":account.public(true)});
    drop(accounts);
    if let Some(c) = state.clients.write().await.get_mut(&id) {
        c.name = name.chars().take(20).collect();
    }
    push_account(&state, &id).await;
    let mut rooms = state.rooms.write().await;
    if let Some(r) = rooms
        .values_mut()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        broadcast(&state, r, &id, "rename", &body).await;
    }
    (StatusCode::OK, Json(out)).into_response()
}
pub async fn watch(
    State(state): State<AppState>,
    axum::extract::Query(q): axum::extract::Query<super::WatchQuery>,
) -> axum::response::Response {
    let id = q.id.unwrap_or_default();
    let token = q.token.unwrap_or_default();
    if !is_client_id(&id) || !is_token(&token) {
        return error(StatusCode::BAD_REQUEST, "Нужны id и токен сессии");
    };
    if state.sessions.read().await.get(&id) != Some(&token) {
        return error(StatusCode::FORBIDDEN, "Неверный токен сессии");
    };
    let until = q.until.unwrap_or_else(|| "any".into());
    if !["any", "game", "your-turn", "opponent", "finished"].contains(&until.as_str()) {
        return error(StatusCode::BAD_REQUEST, "Некорректный until");
    };
    let since = q.since.unwrap_or(0);
    let timeout = q.timeout.unwrap_or(20000).clamp(200, 60000);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
    let mut observed = since;
    loop {
        let rooms = state.rooms.read().await;
        let Some(r) = rooms
            .values()
            .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
        else {
            return Json(json!({"hit":false,"until":until,"since":since,"seq":0,"roomId":null,"events":[],"state":null})).into_response();
        };
        let accounts = state.accounts.read().await;
        let clients = state.clients.read().await;
        let connected = clients.keys().cloned().collect();
        let snap = snapshot(r, &id, &accounts, &connected);
        let matches = match until.as_str() {
            "your-turn" => [
                "canAttack",
                "canDefend",
                "canTransfer",
                "canTake",
                "canPass",
                "canStart",
            ]
            .iter()
            .any(|k| snap[*k] == true),
            "finished" => r.state.status == durak_engine::Status::Finished,
            "opponent" => r.events.iter().any(|e| {
                e["seq"].as_u64().unwrap_or(0) > since
                    && e["actorId"].as_str().is_some_and(|actor| actor != id)
            }),
            _ => true,
        };
        let hit = r.seq > since && matches;
        let expired = tokio::time::Instant::now() >= deadline;
        if hit || expired {
            return Json(json!({"hit":hit,"until":until,"since":since,"seq":r.seq,"roomId":r.id,"events":r.events.iter().rev().take(12).rev().collect::<Vec<_>>(),"state":snap})).into_response();
        };
        let rid = r.id.clone();
        observed = observed.max(r.seq);
        drop(clients);
        drop(accounts);
        drop(rooms);
        if tokio::time::timeout_at(deadline, state.event_hub.wait_for_change(&rid, observed))
            .await
            .is_err()
        {
            continue;
        }
    }
}

pub async fn admin(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> axum::response::Response {
    if body.get("clientId").is_some() {
        if let Err(e) = auth(&state, &body).await {
            return e;
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
        return Json(json!({"ok":true,"players":accounts.values().map(|a|a.public(clients.contains_key(&a.id))).collect::<Vec<_>>()})).into_response();
    };
    let id = body.get("id").and_then(Value::as_str).unwrap_or("");
    let mut accounts = state.accounts.write().await;
    let Some(a) = accounts.get_mut(id) else {
        return error(StatusCode::NOT_FOUND, "Игрок не найден");
    };
    match op {
        "reset" => {
            a.balance = 1000;
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
                a.name = name.chars().take(20).collect();
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
    let out = a.public(state.clients.read().await.contains_key(id));
    drop(accounts);
    push_account(&state, id).await;
    Json(json!({"ok":true,"player":out})).into_response()
}
pub async fn connected(state: &AppState, id: &str) {
    let mut rooms = state.rooms.write().await;
    if let Some(r) = rooms
        .values_mut()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        broadcast(state, r, id, "connect", &json!({})).await;
    } else if let Some(c) = state.clients.read().await.get(id) {
        let _ =
            c.sender
                .try_send(Ok(axum::response::sse::Event::default()
                    .data(json!({"type":"noroom"}).to_string())));
    }
    let accounts = state.accounts.read().await;
    let lobby = json!({"type":"lobby","online":state.clients.read().await.len(),"rooms":rooms.values().map(|r|room_summary(r,&accounts)).collect::<Vec<_>>(),"history":[]});
    for c in state.clients.read().await.values() {
        let _ = c.sender.try_send(Ok(
            axum::response::sse::Event::default().data(lobby.to_string())
        ));
    }
}
pub async fn disconnected(state: &AppState, id: &str) {
    let mut rooms = state.rooms.write().await;
    if let Some(r) = rooms
        .values_mut()
        .find(|r| r.state.players.iter().any(|p| p.id == id && !p.left))
    {
        broadcast(state, r, id, "disconnect", &json!({})).await;
    }
}
pub async fn api_status(state: &AppState) -> impl IntoResponse {
    let rooms = state.rooms.read().await;
    Json(
        json!({"online":state.clients.read().await.len(),"clients":[],"rooms":rooms.values().map(|r|json!({"id":r.id,"status":status(r.state.status),"bet":r.bet,"pot":r.pot,"players":r.state.players.len()})).collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn starting_attacker_cannot_take_from_an_empty_table() {
        let mut game = DurakState::new(
            vec!["a".into(), "b".into()],
            Mode::Classic,
            DeckSize::ThirtySix,
        );
        game.start(&mut durak_engine::Lcg(5)).unwrap();
        let attacker = game.attacker_id.clone().unwrap();
        assert!(game.table.is_empty());
        assert!(game.apply(Command { player_id: attacker, action: Action::Take }).is_err());
    }
    #[test]
    fn card_roundtrip_and_reject_invalid() {
        for c in durak_engine::build_deck(DeckSize::FiftyTwo) {
            assert_eq!(card(&card_json(c)), Some(c));
        }
        assert_eq!(card(&json!("bad")), None);
    }
    #[test]
    fn snapshot_hides_other_hands() {
        let mut game = DurakState::new(
            vec!["a".into(), "b".into()],
            Mode::Classic,
            DeckSize::ThirtySix,
        );
        game.start(&mut durak_engine::Lcg(5)).unwrap();
        let r = Room {
            id: "test".into(),
            name: "test".into(),
            mode: Mode::Classic,
            deck_size: DeckSize::ThirtySix,
            bet: 0,
            host: "a".into(),
            state: game,
            seq: 0,
            events: vec![],
            pot: 0,
            payouts: HashMap::new(),
            message: "Ожидаем игроков. Отправьте друзьям код приглашения".into(),
        };
        let view = snapshot(&r, "a", &Default::default(), &Default::default());
        assert_eq!(view["hand"].as_array().unwrap().len(), 6);
        assert!(view["players"][1].get("hand").is_none());
        assert_eq!(
            snapshot(&r, "outsider", &Default::default(), &Default::default())["hand"],
            json!([])
        );
    }
    #[test]
    fn snapshot_exposes_legacy_fields_and_actions() {
        use durak_engine::{Player, Status, TableRow};
        let mut g = DurakState::new(
            vec!["a".into(), "b".into(), "c".into()],
            Mode::Transfer,
            DeckSize::ThirtySix,
        );
        let six = Card::new(Rank::Six, Suit::Spades);
        let six_heart = Card::new(Rank::Six, Suit::Hearts);
        let seven = Card::new(Rank::Seven, Suit::Spades);
        g.status = Status::Playing;
        g.trump = Some(Suit::Clubs);
        g.attacker_id = Some("a".into());
        g.defender_id = Some("b".into());
        g.turn_id = Some("b".into());
        g.players = vec![
            Player {
                id: "a".into(),
                hand: vec![six_heart],
                finished: false,
                left: false,
            },
            Player {
                id: "b".into(),
                hand: vec![six_heart, seven],
                finished: false,
                left: false,
            },
            Player {
                id: "c".into(),
                hand: vec![seven, six],
                finished: false,
                left: false,
            },
        ];
        g.table = vec![TableRow {
            attack: six,
            defense: None,
        }];
        let mut r = Room {
            id: "test".into(),
            name: "room".into(),
            mode: Mode::Transfer,
            deck_size: DeckSize::ThirtySix,
            bet: 0,
            host: "a".into(),
            state: g,
            seq: 1,
            events: (1..=10).map(|n| json!({"seq":n})).collect(),
            pot: 0,
            payouts: HashMap::new(),
            message: String::new(),
        };
        let mut accounts = std::collections::HashMap::new();
        accounts.insert("b".into(), Account::new("b", "Борис"));
        let connected = ["a".into(), "b".into()].into_iter().collect();
        let v = snapshot(&r, "b", &accounts, &connected);
        for key in [
            "room",
            "players",
            "you",
            "hand",
            "trump",
            "trumpCard",
            "deck",
            "table",
            "attackerId",
            "defenderId",
            "turnId",
            "attackerName",
            "defenderName",
            "turnName",
            "message",
            "canAttack",
            "canDefend",
            "canTransfer",
            "canPass",
            "canTake",
            "takePending",
            "canStart",
            "payout",
            "finished",
            "seq",
            "log",
        ] {
            assert!(v.get(key).is_some(), "missing {key}");
        }
        assert_eq!(v["turnName"], "Борис");
        assert_eq!(v["canDefend"], true);
        assert_eq!(v["canTake"], true);
        assert_eq!(v["canTransfer"], true);
        assert_eq!(v["you"]["turn"], true);
        assert_eq!(v["players"][2]["connected"], false);
        assert_eq!(v["log"].as_array().unwrap().len(), 8);
        let a = snapshot(&r, "a", &accounts, &connected);
        assert_eq!(a["canAttack"], true);
        assert_eq!(a["canPass"], false);
        r.state.take_pending = true;
        let a = snapshot(&r, "a", &accounts, &connected);
        assert_eq!(a["canPass"], true);
        let b = snapshot(&r, "b", &accounts, &connected);
        assert_eq!(b["canDefend"], false);
        assert_eq!(b["canTransfer"], false);
        r.state.status = Status::Waiting;
        let a = snapshot(&r, "a", &accounts, &connected);
        assert_eq!(a["canStart"], true);
        assert_eq!(a["canAttack"], false);
    }
}
