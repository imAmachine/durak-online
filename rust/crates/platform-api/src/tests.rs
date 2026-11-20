use super::*;
use axum::{body::Body, http::Request, response::Response};
use tempfile::tempdir;
use tower::ServiceExt;

struct TestStream {
    stream: axum::body::BodyDataStream,
    buffer: String,
}

async fn open_events(app: &Router, id: &str) -> TestStream {
    let token = "a".repeat(64);
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/events?id={id}&token={token}&name={id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    TestStream {
        stream: response.into_body().into_data_stream(),
        buffer: String::new(),
    }
}

async fn lobby(
    stream: &mut TestStream,
    predicate: impl Fn(&serde_json::Value) -> bool,
) -> serde_json::Value {
    use tokio_stream::StreamExt;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            while let Some(end) = stream.buffer.find("\n\n") {
                let frame = stream.buffer[..end].to_owned();
                stream.buffer.drain(..end + 2);
                if let Some(data) = frame.lines().find_map(|line| line.strip_prefix("data: ")) {
                    let value: serde_json::Value = serde_json::from_str(data).unwrap();
                    if value["type"] == "lobby" && predicate(&value) {
                        return value;
                    }
                }
            }
            let bytes = stream.stream.next().await.unwrap().unwrap();
            stream.buffer.push_str(std::str::from_utf8(&bytes).unwrap());
        }
    })
    .await
    .expect("lobby update timed out")
}

async fn post(app: &Router, path: &str, value: serde_json::Value) -> serde_json::Value {
    let response = app
        .clone()
        .oneshot(
            Request::post(path)
                .header("content-type", "application/json")
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn player_profile_only_shares_stats_with_tablemates() {
    let directory = tempdir().unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let repository =
        storage::Repository::open(directory.path().join("profiles.sqlite"), migrations).unwrap();
    let state = AppState::with_repository(repository).unwrap();
    let app = router_with_state(state.clone());
    let _host = open_events(&app, "host").await;
    let _guest = open_events(&app, "guest").await;
    let _third = open_events(&app, "third").await;
    let token = "a".repeat(64);
    let room = post(&app, "/api/rooms", json!({"clientId":"host","token":token})).await;
    let code = room["code"].as_str().unwrap();
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code}),
    )
    .await;
    {
        let mut accounts = state.accounts.write().await;
        let guest = accounts.get_mut("guest").unwrap();
        guest.balance = 0;
        guest.games = 5;
        guest.wins = 2;
        guest.losses = 3;
    }
    let stats = post(
        &app,
        "/api/player-profile",
        json!({"clientId":"host","token":token,"roomId":code,"playerId":"guest"}),
    )
    .await;
    assert_eq!(stats, json!({"balance":0,"games":5,"wins":2,"losses":3}));
    for (viewer, viewer_token, player, status) in [
        ("third", token.clone(), "guest", StatusCode::NOT_FOUND),
        ("host", token, "third", StatusCode::NOT_FOUND),
        ("host", "b".repeat(64), "guest", StatusCode::FORBIDDEN),
    ] {
        let request = Request::post("/api/player-profile")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"clientId":viewer,"token":viewer_token,"roomId":code,"playerId":player})
                    .to_string(),
            ))
            .unwrap();
        assert_eq!(app.clone().oneshot(request).await.unwrap().status(), status);
    }
}

#[tokio::test]
async fn admin_creates_rotates_and_deletes_temporary_accounts() {
    std::env::set_var("ADMIN_KEY", "panel-secret");
    let dir = tempdir().unwrap();
    let path = dir.path().join("admin.sqlite");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    let app = router_with_state(state.clone());
    let created = post(
        &app,
        "/api/admin",
        json!({"key":"panel-secret","op":"create_temp","name":"Гость","password":"первый-пароль"}),
    )
    .await;
    let id = created["account"]["id"].as_str().unwrap().to_owned();
    let stored = state
        .repository
        .as_ref()
        .unwrap()
        .lock()
        .await
        .load_account(&id)
        .unwrap()
        .unwrap();
    let hash = stored.password_hash.clone().unwrap();
    assert!(hash.starts_with("pbkdf2_sha256$") && !hash.contains("первый-пароль"));
    let login = post(
        &app,
        "/api/auth/temp/login",
        json!({"name":"гость","password":"первый-пароль"}),
    )
    .await;
    assert_eq!(login["clientId"], id);
    let token = login["token"].as_str().unwrap().to_owned();
    let stream = app
        .clone()
        .oneshot(
            Request::get(format!("/events?id={id}&token={token}&name=Гость"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stream.status(), StatusCode::OK);
    let changed = post(
        &app,
        "/api/admin",
        json!({"key":"panel-secret","op":"update","id":id,"password":"второй-пароль"}),
    )
    .await;
    assert_eq!(changed["sessionRevoked"], true);
    assert!(!state.sessions.read().await.contains_key(&id));
    assert!(state.clients.read().await.contains_key(&id));
    let stale = app
        .clone()
        .oneshot(
            Request::post("/api/auth/temp/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name":"Гость","password":"первый-пароль"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::FORBIDDEN);
    let list = post(
        &app,
        "/api/admin",
        json!({"key":"panel-secret","op":"list"}),
    )
    .await;
    let row = list["players"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == id.as_str())
        .cloned()
        .unwrap();
    assert_eq!(row["kind"], "temp");
    assert_eq!(row["hasPassword"], true);
    let login = post(
        &app,
        "/api/auth/temp/login",
        json!({"name":"Гость","password":"второй-пароль"}),
    )
    .await;
    let token = login["token"].as_str().unwrap();
    let active_stream = app
        .clone()
        .oneshot(
            Request::get(format!("/events?id={id}&token={token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(active_stream.status(), StatusCode::OK);
    let _guest = open_events(&app, "guest").await;
    let room = post(
        &app,
        "/api/rooms",
        json!({"clientId":id,"token":token,"bet":50}),
    )
    .await;
    let code = room["code"].as_str().unwrap();
    let guest_token = "a".repeat(64);
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":guest_token,"roomId":code}),
    )
    .await;
    post(
        &app,
        "/api/action",
        json!({"clientId":id,"token":token,"action":"start"}),
    )
    .await;
    let deleted = post(
        &app,
        "/api/admin",
        json!({"key":"panel-secret","op":"delete","id":id}),
    )
    .await;
    assert_eq!(deleted["deleted"], id.as_str());
    assert!(!state.accounts.read().await.contains_key(&id));
    assert!(state
        .repository
        .as_ref()
        .unwrap()
        .lock()
        .await
        .load_account(&id)
        .unwrap()
        .is_none());
    assert!(!state.sessions.read().await.contains_key(&id));
    let saved = state
        .repository
        .as_ref()
        .unwrap()
        .lock()
        .await
        .load_transition_state(code)
        .unwrap()
        .unwrap();
    assert!(saved.1.contains(&id));
    let left = post(
        &app,
        "/api/leave",
        json!({"clientId":"guest","token":guest_token}),
    )
    .await;
    assert_eq!(left["ok"], true);
    let saved_after = state
        .repository
        .as_ref()
        .unwrap()
        .lock()
        .await
        .load_transition_state(code)
        .unwrap()
        .unwrap();
    assert_eq!(saved_after.0, saved.0 + 1);
    let rejected = app
        .clone()
        .oneshot(
            Request::post("/api/admin")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"key":"panel-secret","op":"delete","id":id}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::NOT_FOUND);
    let wrong = app
        .clone()
        .oneshot(
            Request::post("/api/auth/temp/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"name":"Гость","password":"второй-пароль"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(wrong.status(), StatusCode::FORBIDDEN);
    std::env::remove_var("ADMIN_KEY");
}

#[tokio::test]
async fn lobby_stream_tracks_room_lifecycle_and_connections() {
    let app = router();
    let mut observer = open_events(&app, "observer").await;
    let mut host = open_events(&app, "host").await;
    let guest = open_events(&app, "guest").await;
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"name":"Test"}),
    )
    .await;
    let code = created["code"].as_str().unwrap();
    let room = |v: &serde_json::Value| {
        v["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == code)
            .cloned()
    };
    lobby(&mut observer, |v| {
        room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 1)
    })
    .await;
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code}),
    )
    .await;
    lobby(&mut observer, |v| {
        room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 2)
    })
    .await;
    drop(guest);
    lobby(&mut observer, |v| {
        v["online"] == 2 && room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 1)
    })
    .await;
    let _guest = open_events(&app, "guest").await;
    lobby(&mut observer, |v| {
        room(v).is_some_and(|r| r["players"].as_array().unwrap().len() == 2)
    })
    .await;
    post(
        &app,
        "/api/action",
        json!({"clientId":"host","token":token,"action":"start"}),
    )
    .await;
    lobby(&mut observer, |v| {
        room(v).is_some_and(|r| r["status"] == "playing" && r["players"][0]["cards"] == 6)
    })
    .await;
    post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
    lobby(&mut observer, |v| room(v).is_none()).await;
    lobby(&mut host, |v| room(v).is_none()).await;
}

#[tokio::test]
async fn lobby_removes_empty_room_after_leave() {
    let app = router();
    let mut observer = open_events(&app, "observer").await;
    let _host = open_events(&app, "host").await;
    let token = "a".repeat(64);
    let created = post(&app, "/api/rooms", json!({"clientId":"host","token":token})).await;
    let code = created["code"].as_str().unwrap();
    lobby(&mut observer, |v| {
        v["rooms"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == code)
    })
    .await;
    post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
    lobby(&mut observer, |v| v["rooms"].as_array().unwrap().is_empty()).await;
}

#[tokio::test]
async fn profile_persists_after_reopening_repository() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("profile.sqlite");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let app = router_with_state(
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap(),
    );
    let stream = open_events(&app, "profile-user").await;
    post(
        &app,
        "/api/profile",
        json!({"clientId":"profile-user", "token":"a".repeat(64), "name":"Saved"}),
    )
    .await;
    drop(stream);
    drop(app);
    let repo = storage::Repository::open(&path, &migrations).unwrap();
    assert_eq!(
        repo.load_account("profile-user").unwrap().unwrap().name,
        "Saved"
    );
}

#[tokio::test]
async fn failed_profile_write_does_not_change_memory() {
    let directory = tempdir().unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let repository =
        storage::Repository::open(directory.path().join("failure.sqlite"), migrations).unwrap();
    let state = AppState::with_repository(repository).unwrap();
    let app = router_with_state(state.clone());
    let _stream = open_events(&app, "profile-user").await;
    state.repository.as_ref().unwrap().lock().await.connection().execute_batch("CREATE TRIGGER reject_profile BEFORE UPDATE ON legacy_accounts BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/profile")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"clientId":"profile-user", "token":"a".repeat(64), "name":"Rejected"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_ne!(state.accounts.read().await["profile-user"].name, "Rejected");
}

#[tokio::test]
async fn failed_game_transition_keeps_room_accounts_and_events_unchanged() {
    let directory = tempdir().unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let repository =
        storage::Repository::open(directory.path().join("game.sqlite"), migrations).unwrap();
    let state = AppState::with_repository(repository).unwrap();
    let app = router_with_state(state.clone());
    let _host = open_events(&app, "host").await;
    let _guest = open_events(&app, "guest").await;
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"bet":50}),
    )
    .await;
    let code = created["code"].as_str().unwrap();
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code}),
    )
    .await;
    let before = serde_json::to_value(&state.rooms.read().await[code]).unwrap();
    state.repository.as_ref().unwrap().lock().await.connection().execute_batch("CREATE TRIGGER reject_game BEFORE INSERT ON durable_game_transitions BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/action")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"clientId":"host","token":token,"action":"start"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        serde_json::to_value(&state.rooms.read().await[code]).unwrap(),
        before
    );
    assert_eq!(state.accounts.read().await["host"].balance, 1000);
    assert_eq!(state.accounts.read().await["guest"].balance, 1000);
    let repo = state.repository.as_ref().unwrap().lock().await;
    assert_eq!(repo.load_account("host").unwrap().unwrap().balance, 1000);
    assert_eq!(repo.load_account("guest").unwrap().unwrap().balance, 1000);
    let (_, stored) = repo.load_transition_state(code).unwrap().unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&stored).unwrap(),
        before
    );
}

#[tokio::test]
async fn restarted_game_restores_offline_players_and_settles_once() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("restart.sqlite");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    let app = router_with_state(state.clone());
    let host = open_events(&app, "host").await;
    let guest = open_events(&app, "guest").await;
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"bet":50}),
    )
    .await;
    let code = created["code"].as_str().unwrap().to_string();
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code}),
    )
    .await;
    post(
        &app,
        "/api/action",
        json!({"clientId":"host","token":token,"action":"start"}),
    )
    .await;
    drop(host);
    drop(guest);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.clients.read().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let _guard = state.rooms.write().await;
    })
    .await
    .unwrap();
    let before = serde_json::to_value(&state.rooms.read().await[&code]).unwrap();
    drop(app);
    drop(state);
    let restored =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    assert_eq!(
        serde_json::to_value(&restored.rooms.read().await[&code]).unwrap(),
        before
    );
    assert!(restored.clients.read().await.is_empty());
    assert_eq!(restored.accounts.read().await["guest"].balance, 950);
    let restarted_app = router_with_state(restored.clone());
    let _restored_host = open_events(&restarted_app, "host").await;
    post(
        &restarted_app,
        "/api/leave",
        json!({"clientId":"host","token":token}),
    )
    .await;
    assert_eq!(restored.accounts.read().await["host"].balance, 950);
    assert_eq!(restored.accounts.read().await["guest"].balance, 1050);
    assert_eq!(restored.accounts.read().await["guest"].wins, 1);
    let again =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    assert_eq!(again.accounts.read().await["guest"].balance, 1050);
    assert_eq!(again.accounts.read().await["guest"].games, 1);
    assert_eq!(
        again.rooms.read().await[&code].state.status,
        durak_engine::Status::Finished
    );
}

#[tokio::test]
async fn new_accounts_create_join_and_reopen_with_matching_sequences() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("new.sqlite");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    let app = router_with_state(state.clone());
    let host = open_events(&app, "host").await;
    let guest = open_events(&app, "guest").await;
    state.accounts.write().await.remove("host");
    state.accounts.write().await.remove("guest");
    state
        .repository
        .as_ref()
        .unwrap()
        .lock()
        .await
        .connection()
        .execute(
            "DELETE FROM legacy_accounts WHERE client_id IN ('host','guest')",
            [],
        )
        .unwrap();
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"name":"Saved table","playerName":"Alice"}),
    )
    .await;
    let code = created["code"].as_str().unwrap().to_owned();
    assert_eq!(created["room"]["id"], code);
    assert_eq!(created["room"]["name"], "Saved table");
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code,"playerName":"Bob"}),
    )
    .await;
    let room = state.rooms.read().await[&code].clone();
    let repo = state.repository.as_ref().unwrap().lock().await;
    let saved = repo.load_transition_snapshot(&code).unwrap().unwrap();
    assert_eq!(saved.seq as u64 + 1, room.seq);
    let payload: serde_json::Value = serde_json::from_str(&saved.state_json).unwrap();
    assert_eq!(payload["seq"], room.seq);
    assert_eq!(
        payload["events"].as_array().unwrap().last().unwrap()["seq"],
        room.seq
    );
    assert_eq!(repo.load_account("host").unwrap().unwrap().name, "Alice");
    assert_eq!(repo.load_account("guest").unwrap().unwrap().name, "Bob");
    drop(repo);
    drop(host);
    drop(guest);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.clients.read().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let _guard = state.rooms.write().await;
    })
    .await
    .unwrap();
    drop(app);
    drop(state);
    let restored =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    assert_eq!(restored.rooms.read().await[&code].name, "Saved table");
    assert_eq!(restored.accounts.read().await["host"].name, "Alice");
    assert_eq!(restored.accounts.read().await["guest"].name, "Bob");
}

#[tokio::test]
async fn stale_player_name_from_client_does_not_rename_account() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("names.sqlite");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    let app = router_with_state(state.clone());
    let host = open_events(&app, "host").await;
    let guest = open_events(&app, "guest").await;
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"name":"Стол","playerName":"Игрок"}),
    )
    .await;
    let code = created["code"].as_str().unwrap().to_owned();
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code,"playerName":"Игрок"}),
    )
    .await;
    assert_eq!(state.accounts.read().await["host"].name, "host");
    assert_eq!(state.accounts.read().await["guest"].name, "guest");
    assert_eq!(
        state
            .repository
            .as_ref()
            .unwrap()
            .lock()
            .await
            .load_account("guest")
            .unwrap()
            .unwrap()
            .name,
        "guest"
    );
    drop(host);
    drop(guest);
}

#[tokio::test]
async fn player_can_rejoin_finished_room_and_recover_after_restart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("rejoin.sqlite");
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    let app = router_with_state(state.clone());
    let host = open_events(&app, "host").await;
    let guest = open_events(&app, "guest").await;
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"bet":50}),
    )
    .await;
    let code = created["code"].as_str().unwrap().to_string();
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code}),
    )
    .await;
    post(
        &app,
        "/api/action",
        json!({"clientId":"host","token":token,"action":"start"}),
    )
    .await;
    post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
    let result = post(
        &app,
        "/api/join",
        json!({"clientId":"host","token":token,"roomId":code}),
    )
    .await;
    assert_eq!(result["room"]["id"], code);
    {
        let rooms = state.rooms.read().await;
        let room = &rooms[&code];
        assert_eq!(
            room.state.players.iter().filter(|p| p.id == "host").count(),
            1
        );
        assert!(
            !room
                .state
                .players
                .iter()
                .find(|p| p.id == "host")
                .unwrap()
                .left
        );
        assert_eq!(room.state.status, durak_engine::Status::Finished);
    }
    drop(host);
    drop(guest);
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if state.clients.read().await.is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        let _guard = state.rooms.write().await;
    })
    .await
    .unwrap();
    drop(app);
    drop(state);
    let recovered =
        AppState::with_repository(storage::Repository::open(&path, &migrations).unwrap()).unwrap();
    let rooms = recovered.rooms.read().await;
    let room = &rooms[&code];
    assert_eq!(
        room.state.players.iter().filter(|p| p.id == "host").count(),
        1
    );
    assert!(
        !room
            .state
            .players
            .iter()
            .find(|p| p.id == "host")
            .unwrap()
            .left
    );
    assert_eq!(recovered.accounts.read().await["host"].losses, 1);
    assert_eq!(recovered.accounts.read().await["guest"].wins, 1);
}

#[tokio::test]
async fn finished_game_can_restart_without_repeating_settlement() {
    let dir = tempdir().unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state = AppState::with_repository(
        storage::Repository::open(dir.path().join("finished.sqlite"), migrations).unwrap(),
    )
    .unwrap();
    let app = router_with_state(state.clone());
    let _host = open_events(&app, "host").await;
    let _guest = open_events(&app, "guest").await;
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"bet":50}),
    )
    .await;
    let code = created["code"].as_str().unwrap();
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code}),
    )
    .await;
    post(
        &app,
        "/api/action",
        json!({"clientId":"host","token":token,"action":"start"}),
    )
    .await;
    post(&app, "/api/leave", json!({"clientId":"host","token":token})).await;
    let _third = open_events(&app, "third").await;
    post(
        &app,
        "/api/join",
        json!({"clientId":"third","token":token,"roomId":code}),
    )
    .await;
    post(
        &app,
        "/api/action",
        json!({"clientId":"guest","token":token,"action":"start"}),
    )
    .await;
    let accounts = state.accounts.read().await;
    assert_eq!(accounts["host"].balance, 950);
    assert_eq!(accounts["guest"].balance, 1000);
    assert_eq!(accounts["guest"].games, 1);
    assert_eq!(accounts["guest"].wins, 1);
    assert_eq!(accounts["third"].balance, 950);
    drop(accounts);
    assert_eq!(state.rooms.read().await[code].pot, 100);
    assert_eq!(
        state.rooms.read().await[code].state.status,
        durak_engine::Status::Playing
    );
}

#[tokio::test]
async fn rejected_leave_rolls_back_settlement_and_room() {
    let dir = tempdir().unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let state = AppState::with_repository(
        storage::Repository::open(dir.path().join("leave.sqlite"), migrations).unwrap(),
    )
    .unwrap();
    let app = router_with_state(state.clone());
    let _host = open_events(&app, "host").await;
    let _guest = open_events(&app, "guest").await;
    let token = "a".repeat(64);
    let created = post(
        &app,
        "/api/rooms",
        json!({"clientId":"host","token":token,"bet":50}),
    )
    .await;
    let code = created["code"].as_str().unwrap();
    post(
        &app,
        "/api/join",
        json!({"clientId":"guest","token":token,"roomId":code}),
    )
    .await;
    post(
        &app,
        "/api/action",
        json!({"clientId":"host","token":token,"action":"start"}),
    )
    .await;
    let before = serde_json::to_value(&state.rooms.read().await[code]).unwrap();
    state.repository.as_ref().unwrap().lock().await.connection().execute_batch("CREATE TRIGGER reject_settlement BEFORE UPDATE ON legacy_accounts BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/leave")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"clientId":"host","token":token}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        serde_json::to_value(&state.rooms.read().await[code]).unwrap(),
        before
    );
    assert_eq!(state.accounts.read().await["host"].balance, 950);
    assert_eq!(state.accounts.read().await["guest"].balance, 950);
    assert_eq!(state.accounts.read().await["guest"].games, 0);
    let repo = state.repository.as_ref().unwrap().lock().await;
    assert_eq!(repo.load_account("guest").unwrap().unwrap().balance, 950);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(
            &repo.load_transition_state(code).unwrap().unwrap().1
        )
        .unwrap(),
        before
    );
}

#[test]
fn invalid_recovery_snapshot_prevents_startup() {
    let dir = tempdir().unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let repository =
        storage::Repository::open(dir.path().join("invalid.sqlite"), migrations).unwrap();
    repository
        .connection()
        .execute(
            "INSERT INTO durable_game_states(game_id,seq,state_json) VALUES ('bad',0,'{}')",
            [],
        )
        .unwrap();
    assert!(AppState::with_repository(repository).is_err());
}

#[tokio::test]
async fn readiness_requires_a_migrated_repository() {
    let directory = tempdir().unwrap();
    let repository = storage::Repository::open(
        directory.path().join("durak.sqlite"),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations"),
    )
    .unwrap();
    let response: Response = router_with_state(AppState::with_repository(repository).unwrap())
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response: Response = router_with_state(AppState::default())
        .oneshot(Request::get("/readyz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn unlinked_active_room_fails_closed_at_startup_without_deleting_rows() {
    let dir = tempdir().unwrap();
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let mut repo =
        storage::Repository::open(dir.path().join("orphan.sqlite"), &migrations).unwrap();
    let room = game::Room {
        id: "ORPHAN".into(),
        name: "Locked stakes".into(),
        mode: durak_engine::Mode::Classic,
        deck_size: durak_engine::DeckSize::ThirtySix,
        bet: 50,
        host: "discord:legacy".into(),
        state: durak_engine::DurakState::new(
            vec!["discord:legacy".into(), "discord:ok".into()],
            durak_engine::Mode::Classic,
            durak_engine::DeckSize::ThirtySix,
        ),
        seq: 1,
        events: vec![json!({"seq":1})],
        pot: 100,
        payouts: HashMap::new(),
        message: "playing".into(),
    };
    let transition = storage::DurableTransition {
        game_id: "ORPHAN".into(),
        expected_seq: None,
        seq: 0,
        state_json: serde_json::to_string(&room).unwrap(),
        accounts: vec![],
    };
    repo.commit_transition(&transition).unwrap();
    let result = AppState::with_repository(repo);
    assert!(result.is_err());
}

#[tokio::test]
async fn invalid_events_credentials_are_rejected() {
    let response: Response = router()
        .oneshot(
            Request::get("/events?id=bad&token=bad")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
