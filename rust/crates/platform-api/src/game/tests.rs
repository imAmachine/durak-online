#[test]
fn room_roundtrip_preserves_private_game_state() {
    use super::*;
    let mut game = DurakState::new(
        vec!["a".into(), "b".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.start(&mut durak_engine::Lcg(42)).unwrap();
    let room = Room {
        id: "ABCDEF".into(),
        name: "test".into(),
        mode: Mode::Classic,
        deck_size: DeckSize::ThirtySix,
        bet: 50,
        host: "a".into(),
        state: game.clone(),
        seq: 7,
        events: vec![],
        pot: 100,
        payouts: HashMap::new(),
        message: "playing".into(),
    };
    let encoded = serde_json::to_string(&room).unwrap();
    let restored: Room = serde_json::from_str(&encoded).unwrap();
    assert_eq!(restored.state, game);
    assert_eq!(restored.pot, 100);
    assert_eq!(restored.seq, 7);
}
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
    assert!(game
        .apply(Command {
            player_id: attacker,
            action: Action::Take
        })
        .is_err());
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
    r.state.players[0].hand = vec![six_heart, Card::new(Rank::Nine, Suit::Spades)];
    let a = snapshot(&r, "a", &accounts, &connected);
    assert_eq!(a["canAttack"], true);
    assert_eq!(a["canPass"], false);
    r.state.players[0].hand = vec![six_heart];
    r.state.take_pending = true;
    r.state.turn_id = Some("a".into());
    let a = snapshot(&r, "a", &accounts, &connected);
    assert_eq!(a["canAttack"], true);
    assert_eq!(a["canPass"], true);
    let b = snapshot(&r, "b", &accounts, &connected);
    assert_eq!(b["canDefend"], false);
    assert_eq!(b["canTransfer"], false);
    r.state.take_pending = false;
    r.state.turn_id = Some("a".into());
    r.state.players[0].hand.clear();
    r.state.table = vec![
        TableRow {
            attack: six,
            defense: Some(seven),
        },
        TableRow {
            attack: Card::new(Rank::Nine, Suit::Diamonds),
            defense: Some(Card::new(Rank::Ten, Suit::Diamonds)),
        },
    ];
    let a = snapshot(&r, "a", &accounts, &connected);
    assert_eq!(a["canAttack"], false);
    assert_eq!(
        a["canPass"], true,
        "атакующий с пустой рукой обязан закрыть кон"
    );
    assert_eq!(a["you"]["turn"], true);
    r.state.players[0].hand = vec![six_heart];
    r.state.table = vec![TableRow {
        attack: six,
        defense: None,
    }];
    r.state.turn_id = Some("c".into());
    let c = snapshot(&r, "c", &accounts, &connected);
    assert_eq!(c["canAttack"], true);
    assert_eq!(c["canPass"], false);
    assert_eq!(snapshot(&r, "a", &accounts, &connected)["canPass"], false);
    r.state.take_pending = false;
    r.state.turn_id = Some("c".into());
    r.state.passed_attackers = vec!["a".into()];
    let a = snapshot(&r, "a", &accounts, &connected);
    assert_eq!(a["canAttack"], false);
    assert_eq!(a["canPass"], false);
    r.state.status = Status::Waiting;
    let a = snapshot(&r, "a", &accounts, &connected);
    assert_eq!(a["canStart"], true);
    assert_eq!(a["canAttack"], false);
}
#[tokio::test]
async fn account_creation_time_is_stored_and_listed_for_admin() {
    std::env::set_var("ADMIN_KEY", "panel-secret");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("created.sqlite");
    let migrations = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state = AppState::with_repository(
        super::super::storage::Repository::open(&path, &migrations).unwrap(),
    )
    .unwrap();
    let before = i64::try_from(now() / 1000).unwrap();
    let created = admin(State(state.clone()), Json(json!({"key":"panel-secret","op":"create_temp","name":"Клуб","password":"пароль-клуба"}))).await;
    let body = axum::body::to_bytes(created.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body: Value = serde_json::from_slice(&body).unwrap();
    let id = body["account"]["id"].as_str().unwrap().to_owned();
    let created_at = body["account"]["createdAt"].as_i64().unwrap();
    assert!(created_at >= before);
    let stored = state
        .repository
        .as_ref()
        .unwrap()
        .lock()
        .await
        .load_account(&id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.created_at, Some(created_at));
    let listed = admin_view(&state.accounts.read().await[&id], false);
    assert_eq!(listed["createdAt"], json!(created_at));
}

#[tokio::test]
async fn offline_cleanup_refunds_all_players_once_without_recording_games() {
    let state = AppState::default();
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = durak_engine::Status::Playing;
    game.players[0].finished = true;
    game.players[0].left = true;
    let room = Room {
        id: "room".into(),
        name: "test".into(),
        mode: Mode::Classic,
        deck_size: DeckSize::ThirtySix,
        bet: 100,
        host: "a".into(),
        state: game,
        seq: 0,
        events: vec![],
        pot: 300,
        payouts: HashMap::new(),
        message: String::new(),
    };
    state.rooms.write().await.insert(room.id.clone(), room);
    for id in ["a", "b", "c"] {
        let mut account = Account::new(id, id);
        account.balance = 900;
        state.accounts.write().await.insert(id.into(), account);
    }
    state
        .offline_rooms
        .write()
        .await
        .insert("room".into(), now());
    cleanup_rooms(&state).await;
    assert!(state.rooms.read().await.contains_key("room"));
    state
        .offline_rooms
        .write()
        .await
        .insert("room".into(), now() - OFFLINE_GRACE_MS);
    cleanup_rooms(&state).await;
    assert!(!state.rooms.read().await.contains_key("room"));
    cleanup_rooms(&state).await;
    let accounts = state.accounts.read().await;
    for id in ["a", "b", "c"] {
        assert_eq!(accounts[id].balance, 1000);
        assert_eq!(
            (accounts[id].games, accounts[id].wins, accounts[id].losses),
            (0, 0, 0)
        );
    }
}

#[tokio::test]
async fn leaving_active_or_finished_player_settles_as_expected() {
    for finished in [false, true] {
        let state = AppState::default();
        let mut game = DurakState::new(
            vec!["a".into(), "b".into(), "c".into()],
            Mode::Classic,
            DeckSize::ThirtySix,
        );
        game.status = durak_engine::Status::Playing;
        game.players[0].finished = finished;
        let room = Room {
            id: "room".into(),
            name: "test".into(),
            mode: Mode::Classic,
            deck_size: DeckSize::ThirtySix,
            bet: 100,
            host: "a".into(),
            state: game,
            seq: 0,
            events: vec![],
            pot: 300,
            payouts: HashMap::new(),
            message: String::new(),
        };
        let mut rooms = HashMap::from([("room".into(), room)]);
        for id in ["a", "b", "c"] {
            let mut account = Account::new(id, id);
            account.balance = 900;
            state.accounts.write().await.insert(id.into(), account);
        }
        leave_inner(&state, &mut rooms, "room", "a", false)
            .await
            .unwrap();
        let room = &rooms["room"];
        assert_eq!(room.state.players.len(), 3);
        assert!(room.state.players[0].left);
        let accounts = state.accounts.read().await;
        if finished {
            assert_eq!(room.state.status, durak_engine::Status::Playing);
            assert_eq!(room.pot, 300);
            assert_eq!(accounts["a"].games, 0);
        } else {
            assert_eq!(room.state.status, durak_engine::Status::Finished);
            assert_eq!(
                room.state.finished.as_ref().unwrap().loser_id.as_deref(),
                Some("a")
            );
            assert_eq!(room.pot, 0);
            assert_eq!(accounts["a"].balance, 900);
            assert_eq!(accounts["a"].losses, 1);
            assert_eq!(accounts["b"].balance, 1050);
            assert_eq!(accounts["c"].balance, 1050);
        }
    }
}

#[tokio::test]
async fn gameplay_auth_rejects_missing_wrong_and_malformed_tokens() {
    let state = AppState::default();
    let token = "a".repeat(64);
    state
        .sessions
        .write()
        .await
        .insert("player".into(), token.clone());
    for body in [
        json!({"clientId":"player"}),
        json!({"clientId":"player", "token":"b".repeat(64)}),
        json!({"clientId":"player", "token":"A".repeat(64)}),
    ] {
        assert_eq!(
            auth(&state, &body).await.unwrap_err().status(),
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        auth(&state, &json!({"clientId":"player", "token":token}))
            .await
            .unwrap(),
        "player"
    );
}

async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(2), future)
        .await
        .expect("game test timed out")
}

async fn test_client(state: &AppState, id: &str) {
    let (sender, _receiver) = tokio::sync::mpsc::channel(16);
    state.clients.write().await.insert(
        id.into(),
        super::super::ClientEntry {
            generation: 1,
            sender,
            name: id.into(),
            cancel: std::sync::Mutex::new(None),
        },
    );
}

fn test_room(players: Vec<&str>, status: durak_engine::Status, bet: i64) -> Room {
    let ids = players.into_iter().map(str::to_owned).collect();
    let mut state = DurakState::new(ids, Mode::Classic, DeckSize::ThirtySix);
    state.status = status;
    Room {
        id: "ROOM".into(),
        name: "test".into(),
        mode: Mode::Classic,
        deck_size: DeckSize::ThirtySix,
        bet,
        host: "host".into(),
        state,
        seq: 0,
        events: vec![],
        pot: 0,
        payouts: HashMap::new(),
        message: String::new(),
    }
}

#[tokio::test]
async fn join_rejected_during_play_preserves_room() {
    let state = AppState::default();
    let token = "a".repeat(64);
    state
        .sessions
        .write()
        .await
        .insert("guest".into(), token.clone());
    bounded(test_client(&state, "guest")).await;
    state.rooms.write().await.insert(
        "ROOM".into(),
        test_room(vec!["host"], durak_engine::Status::Playing, 100),
    );
    let before = serde_json::to_value(&state.rooms.read().await["ROOM"]).unwrap();
    let response = bounded(join(
        State(state.clone()),
        Json(json!({"clientId":"guest","token":token,"roomId":"ROOM"})),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::to_value(&state.rooms.read().await["ROOM"]).unwrap(),
        before
    );
}

#[tokio::test]
async fn join_rejected_when_six_seats_are_reserved() {
    let state = AppState::default();
    let token = "a".repeat(64);
    state
        .sessions
        .write()
        .await
        .insert("guest".into(), token.clone());
    bounded(test_client(&state, "guest")).await;
    state.rooms.write().await.insert(
        "ROOM".into(),
        test_room(
            vec!["host", "p1", "p2", "p3", "p4", "p5"],
            durak_engine::Status::Waiting,
            0,
        ),
    );
    let before = serde_json::to_value(&state.rooms.read().await["ROOM"]).unwrap();
    let response = bounded(join(
        State(state.clone()),
        Json(json!({"clientId":"guest","token":token,"roomId":"ROOM"})),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::to_value(&state.rooms.read().await["ROOM"]).unwrap(),
        before
    );
}

#[tokio::test]
async fn join_rejected_for_insufficient_funds_preserves_room() {
    let state = AppState::default();
    let token = "a".repeat(64);
    state
        .sessions
        .write()
        .await
        .insert("guest".into(), token.clone());
    bounded(test_client(&state, "guest")).await;
    let mut guest = Account::new("guest", "guest");
    guest.balance = 99;
    state.accounts.write().await.insert("guest".into(), guest);
    state.rooms.write().await.insert(
        "ROOM".into(),
        test_room(vec!["host"], durak_engine::Status::Waiting, 100),
    );
    let before = serde_json::to_value(&state.rooms.read().await["ROOM"]).unwrap();
    let response = bounded(join(
        State(state.clone()),
        Json(json!({"clientId":"guest","token":token,"roomId":"ROOM"})),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        serde_json::to_value(&state.rooms.read().await["ROOM"]).unwrap(),
        before
    );
    assert_eq!(state.accounts.read().await["guest"].balance, 99);
}

#[tokio::test]
async fn reconnect_keeps_existing_seat_and_publishes_current_game() {
    let state = AppState::default();
    let token = "a".repeat(64);
    state
        .sessions
        .write()
        .await
        .insert("guest".into(), token.clone());
    bounded(test_client(&state, "guest")).await;
    let mut room = test_room(vec!["host", "guest"], durak_engine::Status::Playing, 100);
    room.state.players[1].hand = vec![Card::new(Rank::Six, Suit::Spades)];
    state.rooms.write().await.insert("ROOM".into(), room);
    bounded(disconnected(&state, "guest")).await;
    let response = bounded(join(
        State(state.clone()),
        Json(json!({"clientId":"guest","token":token,"roomId":"ROOM"})),
    ))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    {
        let rooms = state.rooms.read().await;
        let room = &rooms["ROOM"];
        assert_eq!(
            room.state
                .players
                .iter()
                .filter(|p| p.id == "guest")
                .count(),
            1
        );
        assert!(
            !room
                .state
                .players
                .iter()
                .find(|p| p.id == "guest")
                .unwrap()
                .left
        );
        assert_eq!(room.state.status, durak_engine::Status::Playing);
    }
    bounded(connected(&state, "guest")).await;
}

#[tokio::test]
async fn draw_returns_stakes_and_records_zero_net_payouts() {
    let state = AppState::default();
    let token = "a".repeat(64);
    for id in ["host", "guest"] {
        state
            .sessions
            .write()
            .await
            .insert(id.into(), token.clone());
        bounded(test_client(&state, id)).await;
        let mut account = Account::new(id, id);
        account.balance = 900;
        state.accounts.write().await.insert(id.into(), account);
    }
    let attack = Card::new(Rank::Six, Suit::Spades);
    let defense = Card::new(Rank::Seven, Suit::Spades);
    let mut game = DurakState::new(
        vec!["host".into(), "guest".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = durak_engine::Status::Playing;
    game.trump = Some(Suit::Clubs);
    game.attacker_id = Some("host".into());
    game.defender_id = Some("guest".into());
    game.turn_id = Some("host".into());
    game.players[0].hand = vec![attack];
    game.players[1].hand = vec![defense];
    state.rooms.write().await.insert(
        "ROOM".into(),
        Room {
            id: "ROOM".into(),
            name: "test".into(),
            mode: Mode::Classic,
            deck_size: DeckSize::ThirtySix,
            bet: 100,
            host: "host".into(),
            state: game,
            seq: 0,
            events: vec![],
            pot: 200,
            payouts: HashMap::new(),
            message: String::new(),
        },
    );
    let play = |action_name: &str, id: &str, card_id: Option<Value>| {
        let mut body = json!({"clientId":id,"token":token,"action":action_name});
        if let Some(card) = card_id {
            body["cardId"] = card;
        }
        bounded(action(State(state.clone()), Json(body)))
    };
    assert_eq!(
        bounded(play("attack", "host", Some(json!("6♠"))))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        bounded(play("defend", "guest", Some(json!("7♠"))))
            .await
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        bounded(play("pass", "host", None)).await.status(),
        StatusCode::OK
    );
    let room = &state.rooms.read().await["ROOM"];
    assert_eq!(room.state.status, durak_engine::Status::Finished);
    assert!(room.state.finished.as_ref().unwrap().draw);
    assert_eq!(room.pot, 0);
    assert_eq!(room.payouts["host"], 0);
    assert_eq!(room.payouts["guest"], 0);
    let accounts = state.accounts.read().await;
    assert_eq!(accounts["host"].balance, 1000);
    assert_eq!(accounts["guest"].balance, 1000);
}
