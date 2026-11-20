use super::*;
use std::{fs, path::PathBuf};
use tempfile::tempdir;
fn migrations() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../migrations")
}
fn setup() -> (Repository, tempfile::TempDir) {
    let d = tempdir().unwrap();
    let mut c = Connection::open(d.path().join("db.sqlite")).unwrap();
    configure(&c).unwrap();
    run_migrations(&mut c, migrations()).unwrap();
    c.execute("INSERT INTO users(id,created_at) VALUES ('u',1)", [])
        .unwrap();
    c.execute("INSERT INTO tables(id,owner_user_id,room_code,mode,status,created_at,updated_at) VALUES ('t','u','ROOM','classic','playing',1,1)",[]).unwrap();
    (Repository::new(c).unwrap(), d)
}
fn state() -> MatchState {
    MatchState {
        id: "m".into(),
        table_id: "t".into(),
        game_type: "durak".into(),
        rules_version: "1".into(),
        mode: "classic".into(),
        status: "active".into(),
        state_json: r#"{"turn":1}"#.into(),
        seq: 1,
        created_at: 1,
        updated_at: 1,
        finished_at: None,
    }
}
fn event() -> GameEvent {
    GameEvent {
        id: "e".into(),
        match_id: "m".into(),
        seq: 1,
        public_json: r#"{"action":"start"}"#.into(),
        created_at: 1,
    }
}
#[test]
fn durable_transition_is_atomic_idempotent_and_survives_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("transition.sqlite");
    let mut repo = Repository::open(&path, migrations()).unwrap();
    let account = crate::game::Account::new("a", "Alice");
    repo.save_account(&account).unwrap();
    let before = DurableAccount {
        id: "a".into(),
        name: "Alice".into(),
        balance: 1000,
        games: 0,
        wins: 0,
        losses: 0,
        avatar_url: None,
        password_hash: None,
        created_at: None,
    };
    let mut after = before.clone();
    after.balance = 900;
    let first = DurableTransition {
        game_id: "game".into(),
        expected_seq: None,
        seq: 0,
        state_json: "waiting".into(),
        accounts: vec![],
    };
    repo.commit_transition(&first).unwrap();
    let next = DurableTransition {
        game_id: "game".into(),
        expected_seq: Some(0),
        seq: 1,
        state_json: "playing".into(),
        accounts: vec![AccountChange {
            before: Some(before.clone()),
            after: after.clone(),
        }],
    };
    repo.commit_transition(&next).unwrap();
    repo.commit_transition(&next).unwrap();
    let mut conflict = next.clone();
    conflict.state_json = "different".into();
    assert!(repo.commit_transition(&conflict).is_err());
    let mut invalid = next.clone();
    invalid.expected_seq = Some(1);
    invalid.seq = 2;
    invalid.accounts[0].before = Some(after.clone());
    invalid.accounts[0].after.balance = -1;
    assert!(repo.commit_transition(&invalid).is_err());
    assert_eq!(
        repo.load_transition_state("game").unwrap(),
        Some((1, "playing".into()))
    );
    assert_eq!(repo.load_account("a").unwrap().unwrap().balance, 900);
    drop(repo);
    let repo = Repository::open(&path, migrations()).unwrap();
    assert_eq!(
        repo.load_transition_state("game").unwrap(),
        Some((1, "playing".into()))
    );
    assert_eq!(repo.load_account("a").unwrap().unwrap().balance, 900);
}
#[test]
fn finished_history_is_atomic_idempotent_and_restored_without_users() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("history.sqlite");
    let history = FinishedGameHistory {
        room: "Комната".into(),
        mode: "classic".into(),
        at: 42,
        loser: "discord:loser".into(),
        bet: 100,
    };
    {
        let mut repo = Repository::open(&path, migrations()).unwrap();
        let transition = DurableTransition {
            game_id: "game".into(),
            expected_seq: None,
            seq: 0,
            state_json: "finished".into(),
            accounts: vec![],
        };
        repo.commit_transition_with_history(&transition, Some(&history))
            .unwrap();
        repo.commit_transition_with_history(&transition, Some(&history))
            .unwrap();
        assert_eq!(repo.list_history(8).unwrap(), vec![history.clone()]);
        assert_eq!(
            repo.connection()
                .query_row("SELECT COUNT(*) FROM legacy_accounts", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        let mut changed = history.clone();
        changed.bet = 200;
        assert!(repo
            .commit_transition_with_history(&transition, Some(&changed))
            .is_err());
    }
    let repo = Repository::open(&path, migrations()).unwrap();
    assert_eq!(repo.list_history(8).unwrap(), vec![history]);
}

#[test]
fn invalid_history_rolls_back_transition() {
    let dir = tempdir().unwrap();
    let mut repo =
        Repository::open(dir.path().join("history-failure.sqlite"), migrations()).unwrap();
    let transition = DurableTransition {
        game_id: "game".into(),
        expected_seq: None,
        seq: 0,
        state_json: "finished".into(),
        accounts: vec![],
    };
    let invalid = FinishedGameHistory {
        room: "room".into(),
        mode: "unknown".into(),
        at: 1,
        loser: "discord:x".into(),
        bet: 0,
    };
    assert!(repo
        .commit_transition_with_history(&transition, Some(&invalid))
        .is_err());
    assert!(repo.load_transition_state("game").unwrap().is_none());
    assert!(repo.list_history(8).unwrap().is_empty());
}

#[test]
fn history_lists_newest_first_for_reused_room_and_limits_results() {
    let dir = tempdir().unwrap();
    let mut repo = Repository::open(dir.path().join("ordering.sqlite"), migrations()).unwrap();
    for seq in 0..10 {
        let transition = DurableTransition {
            game_id: "game".into(),
            expected_seq: if seq == 0 { None } else { Some(seq - 1) },
            seq,
            state_json: format!("state-{seq}"),
            accounts: vec![],
        };
        let history = FinishedGameHistory {
            room: "room".into(),
            mode: "transfer".into(),
            at: 1,
            loser: format!("player-{seq}"),
            bet: 0,
        };
        repo.commit_transition_with_history(&transition, Some(&history))
            .unwrap();
    }
    let entries = repo.list_history(8).unwrap();
    assert_eq!(entries.len(), 8);
    assert_eq!(entries[0].loser, "player-9");
    assert_eq!(entries[7].loser, "player-2");
    assert!(repo.list_history(0).unwrap().is_empty());
}

#[test]
fn transition_snapshots_list_latest_states_after_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("snapshots.sqlite");
    let mut repo = Repository::open(&path, migrations()).unwrap();
    assert!(repo.list_transition_snapshots().unwrap().is_empty());
    assert_eq!(repo.load_transition_snapshot("missing").unwrap(), None);
    for (game_id, expected_seq, seq, state_json) in [
        ("z", None, 0, "{\"status\":\"waiting\"}"),
        ("a", None, 0, "{\"status\":\"waiting\"}"),
        ("z", Some(0), 1, "{\"status\":\"playing\"}"),
    ] {
        repo.commit_transition(&DurableTransition {
            game_id: game_id.into(),
            expected_seq,
            seq,
            state_json: state_json.into(),
            accounts: vec![],
        })
        .unwrap();
    }
    drop(repo);
    let repo = Repository::open(&path, migrations()).unwrap();
    let expected = vec![
        TransitionState {
            game_id: "a".into(),
            seq: 0,
            state_json: "{\"status\":\"waiting\"}".into(),
        },
        TransitionState {
            game_id: "z".into(),
            seq: 1,
            state_json: "{\"status\":\"playing\"}".into(),
        },
    ];
    assert_eq!(repo.list_transition_snapshots().unwrap(), expected);
    assert_eq!(
        repo.load_transition_snapshot("z").unwrap(),
        Some(expected[1].clone())
    );
    assert_eq!(repo.load_transition_snapshot("missing").unwrap(), None);
}

#[test]
fn verified_authentication_requires_existing_discord_identity() {
    let dir = tempdir().unwrap();
    let mut repo = Repository::open(dir.path().join("verified.sqlite"), migrations()).unwrap();
    assert!(!repo.authenticate_verified("guest", "token").unwrap());
    assert!(!repo.authenticate_verified("discord:fake", "token").unwrap());
    repo.authenticate_legacy("guest", "token").unwrap();
    assert!(!repo.authenticate_verified("guest", "token").unwrap());
    repo.oauth_login("123", "discord:real", "token").unwrap();
    assert!(repo.authenticate_verified("discord:real", "token").unwrap());
    assert!(!repo.authenticate_verified("discord:real", "wrong").unwrap());
    assert_eq!(
        repo.connection()
            .query_row("SELECT COUNT(*) FROM legacy_accounts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn oauth_identity_and_account_survive_reopen() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("oauth.sqlite");
    {
        let mut repo = Repository::open(&path, migrations()).unwrap();
        assert!(!repo
            .authenticate_legacy("discord:first", "attacker")
            .unwrap());
        assert_eq!(
            repo.oauth_login("123", "discord:first", "hash1").unwrap(),
            "discord:first"
        );
        let mut account = crate::game::Account::new("discord:first", "Alice");
        account.balance = 1250;
        repo.save_account(&account).unwrap();
    }
    let mut repo = Repository::open(&path, migrations()).unwrap();
    assert_eq!(
        repo.oauth_login("123", "discord:second", "hash2").unwrap(),
        "discord:first"
    );
    assert!(!repo.authenticate_legacy("discord:first", "hash1").unwrap());
    assert!(repo.authenticate_legacy("discord:first", "hash2").unwrap());
    let account = repo.load_account("discord:first").unwrap().unwrap();
    assert_eq!(account.name, "Alice");
    assert_eq!(account.balance, 1250);
}
#[test]
fn deleting_account_removes_identity_credentials_and_its_finished_games() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("delete.sqlite");
    let mut repo = Repository::open(&path, migrations()).unwrap();
    let mut account = crate::game::Account::new("temp:gone", "Гость");
    account.password_hash = Some("pbkdf2_sha256$1$aa$bb".into());
    repo.insert_temp_account(&account, "hash").unwrap();
    repo.save_account(&account).unwrap();
    let history = FinishedGameHistory {
        room: "Стол".into(),
        mode: "classic".into(),
        at: 1,
        loser: "Гость".into(),
        bet: 0,
    };
    let transition = DurableTransition {
        game_id: "ROOM01".into(),
        expected_seq: None,
        seq: 0,
        state_json: "{\"id\":\"ROOM01\",\"state\":{\"players\":[{\"id\":\"temp:gone\"}]}}".into(),
        accounts: Vec::new(),
    };
    repo.commit_transition_with_history(&transition, Some(&history))
        .unwrap();
    repo.delete_account("temp:gone", &Default::default())
        .unwrap();
    assert!(repo.load_account("temp:gone").unwrap().is_none());
    assert!(!repo.authenticate_verified("temp:gone", "hash").unwrap());
    assert!(
        repo.connection()
            .query_row(
                "SELECT COUNT(*) FROM legacy_identity WHERE client_id='temp:gone'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
            == 0
    );
    assert!(repo.load_transition_state("ROOM01").unwrap().is_none());
    assert!(repo.list_history(8).unwrap().is_empty());
    assert!(repo
        .delete_account("temp:gone", &Default::default())
        .is_ok());
}
#[test]
fn finish_temp_login_replaces_password_without_duplicating_account() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("login.sqlite");
    let mut repo = Repository::open(&path, migrations()).unwrap();
    let mut account = crate::game::Account::new("temp:first", "Гость");
    account.balance = 420;
    repo.insert_temp_account(&account, "pending").unwrap();
    repo.finish_temp_login("temp:first", "token", "pbkdf2_sha256$1$aa$bb")
        .unwrap();
    let stored = repo.load_account("temp:first").unwrap().unwrap();
    assert_eq!(stored.balance, 420);
    assert_eq!(
        stored.password_hash.as_deref(),
        Some("pbkdf2_sha256$1$aa$bb")
    );
    assert!(repo.authenticate_legacy("temp:first", "token").unwrap());
    repo.replace_password("temp:first", "new", true).unwrap();
    assert_eq!(
        repo.load_account("temp:first")
            .unwrap()
            .unwrap()
            .password_hash
            .as_deref(),
        Some("new")
    );
    assert!(
        repo.connection()
            .query_row(
                "SELECT COUNT(*) FROM legacy_identity WHERE client_id='temp:first'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap()
            == 0
    );
}
#[test]
fn legacy_binding_survives_reopen_and_rejects_other_tokens() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("identity.sqlite");
    {
        let mut repo = Repository::open(&path, migrations()).unwrap();
        assert!(repo.authenticate_legacy("client", "digest-a").unwrap());
        assert!(!repo.authenticate_legacy("client", "digest-b").unwrap());
    }
    let mut repo = Repository::open(&path, migrations()).unwrap();
    assert!(repo.authenticate_legacy("client", "digest-a").unwrap());
    assert!(!repo.authenticate_legacy("client", "digest-b").unwrap());
}
#[test]
fn configures_wal_foreign_keys_and_busy_timeout() {
    let d = tempdir().unwrap();
    let c = Connection::open(d.path().join("x")).unwrap();
    configure(&c).unwrap();
    assert_eq!(
        c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
            .unwrap(),
        "wal"
    );
    assert_eq!(
        c.query_row("PRAGMA busy_timeout", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        5000
    );
}
#[test]
fn saves_bundle_atomically_and_replays_idempotently() {
    let (mut r, _) = setup();
    let w = MatchWrite {
        state: state(),
        event: Some(event()),
        ledger: vec![],
    };
    r.save_match(&w).unwrap();
    r.save_match(&w).unwrap();
    assert_eq!(
        r.connection()
            .query_row("SELECT COUNT(*) FROM game_events", [], |x| x
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn conflicting_event_rolls_back_match_update() {
    let (mut r, _) = setup();
    let w = MatchWrite {
        state: state(),
        event: Some(event()),
        ledger: vec![],
    };
    r.save_match(&w).unwrap();
    let mut changed = state();
    changed.seq = 2;
    changed.state_json = "changed".into();
    let mut bad = event();
    bad.public_json = "different".into();
    assert!(matches!(
        r.save_match(&MatchWrite {
            state: changed,
            event: Some(bad),
            ledger: vec![]
        }),
        Err(StorageError::IdempotencyConflict { kind: "event", .. })
    ));
    assert_eq!(
        r.connection()
            .query_row("SELECT seq FROM matches WHERE id='m'", [], |x| x
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn failed_migration_is_rolled_back() {
    let d = tempdir().unwrap();
    fs::write(
        d.path().join("0001_bad.sql"),
        "CREATE TABLE okay(id INTEGER); SELECT no_such_function();",
    )
    .unwrap();
    let mut c = Connection::open_in_memory().unwrap();
    assert!(run_migrations(&mut c, d.path()).is_err());
    assert_eq!(
        c.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name='okay'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
