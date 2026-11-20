use super::*;

fn game(seq: u64, log: Value) -> Value {
    json!({"type":"game","state":{"seq":seq,"room":{"id":"r"},"log":log}})
}

#[test]
fn cache_requires_new_opponent_action() {
    let seat = Arc::new(Mutex::new(Seat {
        id: "me".into(),
        ..Default::default()
    }));
    seat.lock()
        .unwrap()
        .apply_event(game(3, json!([{"seq":3,"actorId":"other"}])));
    mark_seen(&seat);
    seat.lock().unwrap().apply_event(game(
        4,
        json!([{"seq":3,"actorId":"other"},{"seq":4,"actorId":"me"}]),
    ));
    assert!(cached_watch(&seat, "opponent").is_none());
    seat.lock()
        .unwrap()
        .apply_event(game(5, json!([{"seq":5,"actorId":null}])));
    assert!(cached_watch(&seat, "opponent").is_none());
    seat.lock()
        .unwrap()
        .apply_event(game(6, json!([{"seq":6,"actorId":"other"}])));
    assert!(cached_watch(&seat, "opponent").is_some());
}

#[test]
fn sse_wait_requires_new_opponent_action() {
    let seat = Arc::new(Mutex::new(Seat {
        id: "me".into(),
        ..Default::default()
    }));
    seat.lock()
        .unwrap()
        .apply_event(game(3, json!([{"seq":3,"actorId":"other"}])));
    let sender = seat.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        sender.lock().unwrap().apply_event(game(
            4,
            json!([{"seq":3,"actorId":"other"},{"seq":4,"actorId":"me"}]),
        ));
    });
    assert_eq!(wait_sse(&seat, "opponent", 100)["hit"], false);
    worker.join().unwrap();
    let sender = seat.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(20));
        sender
            .lock()
            .unwrap()
            .apply_event(game(5, json!([{"seq":5,"actorId":"other"}])));
    });
    assert_eq!(wait_sse(&seat, "opponent", 300)["hit"], true);
    worker.join().unwrap();
}

#[test]
fn multiline_sse_data_joins_with_newline() {
    let seat = Arc::new(Mutex::new(Seat::default()));
    read_sse(
        std::io::Cursor::new("data: {\ndata: \"type\":\"lobby\"}\n\n"),
        &seat,
    );
    assert_eq!(seat.lock().unwrap().events.len(), 1);
}

#[test]
fn stale_watch_snapshot_does_not_replace_newer_state() {
    let mut seat = Seat::default();
    seat.update_game(json!({"seq":10,"room":{"id":"r"}}));
    let seat = Arc::new(Mutex::new(seat));
    let result = hook_result(
        &seat,
        &json!({"seq":9,"state":{"seq":9,"room":{"id":"r"}}}),
        8,
    );
    assert_eq!(result["state"]["seq"], 10);
    assert_eq!(seat.lock().unwrap().game.as_ref().unwrap()["seq"], 10);
}

#[test]
fn sse_wait_detects_lobby_event_without_room() {
    let seat = Arc::new(Mutex::new(Seat::default()));
    let sender = seat.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        sender.lock().unwrap().apply_event(json!({"type":"lobby"}));
    });
    let value = wait_watch(&seat, "any", 500).unwrap();
    worker.join().unwrap();
    assert_eq!(value["via"], "sse");
    assert_eq!(value["hit"], true);
}
