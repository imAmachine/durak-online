use super::*;

fn card(id: &str) -> Value {
    parse_card(id).unwrap()
}

fn base() -> Value {
    json!({
        "room":{"id":"R","status":"playing","deckSize":36,"mode":"classic","bet":0},
        "you":{"id":"me","role":"attacker","finished":false,"left":false,"hand":[]},
        "hand":[],
        "players":[
            {"id":"me","name":"Я","cards":2,"role":"attacker"},
            {"id":"op","name":"Оп","cards":6,"role":"defender"}
        ],
        "attackerId":"me",
        "defenderId":"op",
        "trump":"♠",
        "deck":24,
        "table":[],
        "canAttack":false,
        "canDefend":false,
        "canTransfer":false,
        "canPass":false,
        "canTake":false,
        "takePending":false,
        "log":[]
    })
}

fn with_hand(st: &mut Value, cards: &[&str]) {
    let hand: Vec<Value> = cards.iter().map(|id| card(id)).collect();
    st["hand"] = json!(hand);
    st["you"]["hand"] = json!(hand);
}

#[test]
fn card_order_and_trump() {
    assert_eq!(value("Т"), Some(12));
    assert_eq!(value("6"), Some(4));
    assert_eq!(card_text(&card("Т♠")), "Т♠");
    assert_eq!(parse_card("ZZ"), None);
}

#[test]
fn beats_matches_same_suit_trump_and_invalid_cases() {
    assert!(beats(&card("6♥"), &card("7♥"), "♠"));
    assert!(!beats(&card("7♥"), &card("6♥"), "♠"));
    assert!(beats(&card("Т♥"), &card("6♠"), "♠"));
    assert!(!beats(&card("6♠"), &card("Т♥"), "♠"));
    assert!(beats(&card("6♠"), &card("Т♠"), "♠"));
}

#[test]
fn rank_order_has_all_thirteen_ranks() {
    let ranks = [
        "2", "3", "4", "5", "6", "7", "8", "9", "10", "В", "Д", "К", "Т",
    ];
    for pair in ranks.windows(2) {
        assert!(value(pair[0]).unwrap() < value(pair[1]).unwrap());
    }
    assert!(value("Т") > value("К"));
    assert!(value("2") < value("6"));
}

#[test]
fn attack_options_match_table_ranks() {
    let mut st = base();
    with_hand(&mut st, &["6♥", "7♦", "8♣"]);
    st["canAttack"] = json!(true);
    st["table"] = json!([{"attack":card("6♦"),"defense":null}]);
    let ids: Vec<String> = attach_options(&st)
        .iter()
        .map(|c| c["id"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(ids, vec!["6♥".to_string()]);
}

#[test]
fn trumps_are_held_back_mid_deck() {
    let mut st = base();
    with_hand(&mut st, &["6♥", "6♠"]);
    st["canAttack"] = json!(true);
    st["table"] = json!([{"attack":card("6♦"),"defense":null}]);
    let ids: Vec<String> = attack_choices(&st)
        .iter()
        .map(|c| c["id"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(ids, vec!["6♥".to_string()]);
    with_hand(&mut st, &["6♠"]);
    st["deck"] = json!(0);
    let ids: Vec<String> = attack_choices(&st)
        .iter()
        .map(|c| c["id"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(ids, vec!["6♠".to_string()]);
    st["deck"] = json!(24);
    assert!(attack_choices(&st).is_empty());
}

#[test]
fn defense_prefers_cheap_non_trump() {
    let mut st = base();
    with_hand(&mut st, &["7♥", "8♥", "7♠"]);
    st["canDefend"] = json!(true);
    st["canTake"] = json!(true);
    st["you"]["id"] = json!("me");
    st["attackerId"] = json!("op");
    st["defenderId"] = json!("me");
    st["table"] = json!([{"attack":card("6♥"),"defense":null}]);
    let chosen = choose_move(&st, None).unwrap();
    assert_eq!(chosen["action"], "defend");
    assert_eq!(chosen["cardId"], "7♥");
    assert_eq!(chosen["targetCardId"], "6♥");
}

#[test]
fn defense_without_target_card_is_not_emitted() {
    let mut st = base();
    with_hand(&mut st, &["7♥"]);
    st["canDefend"] = json!(true);
    st["attackerId"] = json!("op");
    st["defenderId"] = json!("me");
    st["table"] = json!([]);
    assert!(choose_move(&st, None).is_none());
}

#[test]
fn last_trump_is_saved_until_the_deck_empties() {
    let mut st = base();
    with_hand(&mut st, &["6♠"]);
    st["canDefend"] = json!(true);
    st["canTake"] = json!(true);
    st["attackerId"] = json!("op");
    st["defenderId"] = json!("me");
    st["table"] = json!([{"attack":card("Т♥"),"defense":null}]);
    assert_eq!(choose_move(&st, None).unwrap()["action"], "take");
    st["deck"] = json!(0);
    assert_eq!(choose_move(&st, None).unwrap()["action"], "defend");
}

#[test]
fn transfer_wins_when_defense_is_impossible() {
    let mut st = base();
    with_hand(&mut st, &["9♦", "Т♥"]);
    st["room"]["mode"] = json!("transfer");
    st["canTransfer"] = json!(true);
    st["canTake"] = json!(true);
    st["attackerId"] = json!("op");
    st["defenderId"] = json!("me");
    st["table"] = json!([{"attack":card("9♥"),"defense":null}]);
    assert_eq!(transfer_options(&st).len(), 1);
    let chosen = choose_move(&st, None).unwrap();
    assert_eq!(chosen["action"], "transfer");
    assert_eq!(chosen["cardId"], "9♦");
}

#[test]
fn dump_card_when_the_defender_takes() {
    let mut st = base();
    with_hand(&mut st, &["9♦", "9♠"]);
    st["canAttack"] = json!(true);
    st["canPass"] = json!(true);
    st["takePending"] = json!(true);
    st["table"] = json!([{"attack":card("9♥"),"defense":null}]);
    let dumps: Vec<String> = dump_options(&st)
        .iter()
        .map(|c| c["id"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(dumps, vec!["9♦".to_string()]);
    let chosen = choose_move(&st, None).unwrap();
    assert_eq!(chosen["action"], "attack");
    assert_eq!(chosen["cardId"], "9♦");
}

#[test]
fn defender_does_nothing_while_taking() {
    let mut st = base();
    with_hand(&mut st, &["9♦"]);
    st["takePending"] = json!(true);
    st["attackerId"] = json!("op");
    st["defenderId"] = json!("me");
    st["table"] = json!([{"attack":card("9♥"),"defense":null}]);
    assert!(choose_move(&st, None).is_none());
}

#[test]
fn defense_options_are_sorted_by_non_trump_then_rank() {
    let mut st = base();
    with_hand(&mut st, &["8♥", "7♠", "7♥"]);
    st["canDefend"] = json!(true);
    st["table"] = json!([{"attack":card("6♥"),"defense":null}]);
    let ids: Vec<String> = defense_options(&st)
        .iter()
        .map(|(_, c)| c["id"].as_str().unwrap_or("").to_string())
        .collect();
    assert_eq!(ids, vec!["7♥", "8♥", "7♠"]);
}

#[test]
fn defense_is_preferred_over_transfer_when_it_is_cheap() {
    let mut st = base();
    with_hand(&mut st, &["9♦", "Т♥"]);
    st["room"]["mode"] = json!("transfer");
    st["canDefend"] = json!(true);
    st["canTransfer"] = json!(true);
    st["canTake"] = json!(true);
    st["attackerId"] = json!("op");
    st["defenderId"] = json!("me");
    st["table"] = json!([{"attack":card("9♥"),"defense":null}]);
    let chosen = choose_move(&st, None).unwrap();
    assert_eq!(chosen["action"], "defend");
    assert_eq!(chosen["cardId"], "Т♥");
}

#[test]
fn choose_move_ignores_null_and_waiting_states() {
    assert!(choose_move(&Value::Null, None).is_none());
    let mut st = base();
    st["room"]["status"] = json!("waiting");
    assert!(choose_move(&st, None).is_none());
}

#[test]
fn ledger_summary_reports_deck_and_trumps() {
    let mut st = base();
    with_hand(&mut st, &["6♥", "7♥", "8♥", "9♥", "10♥", "В♥"]);
    st["trumpCard"] = json!(card("6♠"));
    let mut ledger = Ledger::default();
    ledger.observe(&st);
    assert!(ledger.summary(&st)["line"]
        .as_str()
        .unwrap()
        .contains("в колоде 24"));
    assert_eq!(ledger.unseen_trumps(&st), 8);
}

#[test]
fn ledger_keeps_taken_cards_in_play() {
    let mut st = base();
    st["trumpCard"] = json!(card("6♠"));
    with_hand(&mut st, &["6♥"]);
    st["table"] = json!([{"attack":card("6♦"),"defense":card("7♦")}]);
    st["log"] = json!([
        {"seq":1,"action":"attack","cardId":"6♦"},
        {"seq":2,"action":"defend","cardId":"7♦"},
        {"seq":3,"action":"take"}
    ]);
    let mut ledger = Ledger::default();
    ledger.observe(&st);
    assert_eq!(ledger.summary(&st)["out"], json!(0));
    assert_eq!(ledger.summary(&st)["unseen"], json!(32));
}

#[test]
fn ledger_counts_hidden_cards() {
    let mut st = base();
    with_hand(&mut st, &["6♥", "7♥", "8♥", "9♥", "10♥", "В♥"]);
    st["trumpCard"] = json!(card("6♠"));
    let mut ledger = Ledger::default();
    ledger.observe(&st);
    let summary = ledger.summary(&st);
    assert_eq!(summary["unseen"], json!(29));
    assert_eq!(summary["trumps"], json!(8));
}

#[test]
fn ledger_marks_a_beaten_table_as_out_of_play() {
    let mut st = base();
    with_hand(&mut st, &["6♥"]);
    st["table"] = json!([{"attack":card("6♦"),"defense":card("7♦")}]);
    st["log"] = json!([
        {"seq":1,"action":"attack","cardId":"6♦"},
        {"seq":2,"action":"defend","cardId":"7♦","targetCardId":"6♦"}
    ]);
    let mut ledger = Ledger::default();
    ledger.observe(&st);
    st["table"] = json!([]);
    st["log"] = json!([
        {"seq":1,"action":"attack","cardId":"6♦"},
        {"seq":2,"action":"defend","cardId":"7♦","targetCardId":"6♦"},
        {"seq":3,"action":"pass"}
    ]);
    ledger.observe(&st);
    assert_eq!(ledger.summary(&st)["out"], json!(2));
    assert_eq!(
        ledger.dead,
        ["6♦", "7♦"].into_iter().map(str::to_string).collect()
    );
}

#[test]
fn ledger_resets_for_a_new_room() {
    let mut st = base();
    with_hand(&mut st, &["6♥"]);
    let mut ledger = Ledger::default();
    ledger.observe(&st);
    let mut other = base();
    other["room"]["id"] = json!("OTHER");
    other["room"]["deckSize"] = json!(52);
    with_hand(&mut other, &["6♥"]);
    other["trumpCard"] = json!(card("7♣"));
    ledger.observe(&other);
    assert_eq!(ledger.summary(&other)["unseen"], json!(50));
}
