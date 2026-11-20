use crate::{Action, DeckSize, DurakState, Mode, Rank, RuleError, Status, Suit};

use super::{c, cmd};

#[test]
fn throw_in_priority_passes_clockwise_and_obeys_capacity() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into(), "d".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Clubs);
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.trick_attacker_id = Some("a".into());
    let attack = c(Rank::Six, Suit::Spades);
    let extra = c(Rank::Six, Suit::Hearts);
    game.players[0].hand = vec![attack];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades), c(Rank::Eight, Suit::Spades)];
    game.players[2].hand = vec![extra];
    game.players[3].hand = vec![c(Rank::Six, Suit::Diamonds)];
    game = game.apply(cmd("a", Action::Attack(attack))).unwrap().state;
    assert_eq!(
        game.apply(cmd("c", Action::Attack(extra))),
        Err(RuleError::WrongTurn)
    );
    game = game.apply(cmd("b", Action::Take)).unwrap().state;
    assert_eq!(
        game.apply(cmd("c", Action::Attack(extra))),
        Err(RuleError::WrongTurn)
    );
    game = game.apply(cmd("a", Action::Pass)).unwrap().state;
    assert_eq!(game.turn_id.as_deref(), Some("c"));
    game = game.apply(cmd("c", Action::Attack(extra))).unwrap().state;
    assert_eq!(
        game.apply(cmd("c", Action::Attack(extra))),
        Err(RuleError::InsufficientCapacity)
    );
    game = game.apply(cmd("c", Action::Pass)).unwrap().state;
    assert_eq!(game.turn_id.as_deref(), Some("d"));
    game = game.apply(cmd("d", Action::Pass)).unwrap().state;
    assert!(game.table.is_empty());
    assert_eq!(game.players[1].hand.len(), 4);
}
#[test]
fn defended_throw_ins_follow_pass_order() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into(), "d".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Clubs);
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.trick_attacker_id = Some("a".into());
    let six = c(Rank::Six, Suit::Spades);
    let seven = c(Rank::Seven, Suit::Spades);
    let extra = c(Rank::Six, Suit::Hearts);
    game.players[0].hand = vec![six, c(Rank::Nine, Suit::Hearts)];
    game.players[1].hand = vec![
        seven,
        c(Rank::Seven, Suit::Hearts),
        c(Rank::Ten, Suit::Hearts),
    ];
    game.players[2].hand = vec![extra, c(Rank::Eight, Suit::Clubs)];
    game.players[3].hand = vec![c(Rank::Nine, Suit::Clubs)];
    game = game.apply(cmd("a", Action::Attack(six))).unwrap().state;
    game = game
        .apply(cmd(
            "b",
            Action::Defend {
                card: seven,
                target: None,
            },
        ))
        .unwrap()
        .state;
    assert_eq!(
        game.apply(cmd("c", Action::Attack(extra))),
        Err(RuleError::WrongTurn)
    );
    game = game.apply(cmd("a", Action::Pass)).unwrap().state;
    assert_eq!(game.turn_id.as_deref(), Some("c"));
    game = game.apply(cmd("c", Action::Attack(extra))).unwrap().state;
    assert_eq!(game.turn_id.as_deref(), Some("b"));
    game = game
        .apply(cmd(
            "b",
            Action::Defend {
                card: c(Rank::Seven, Suit::Hearts),
                target: None,
            },
        ))
        .unwrap()
        .state;
    assert_eq!(game.turn_id.as_deref(), Some("c"));
    assert_eq!(
        game.apply(cmd("a", Action::Pass)),
        Err(RuleError::WrongTurn)
    );
    game = game.apply(cmd("c", Action::Pass)).unwrap().state;
    assert_eq!(game.turn_id.as_deref(), Some("d"));
    game = game.apply(cmd("d", Action::Pass)).unwrap().state;
    assert!(game.table.is_empty());
    assert_eq!(game.discard.len(), 4);
}
#[test]
fn last_throw_in_card_does_not_restore_passed_attacker() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Clubs);
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.trick_attacker_id = Some("a".into());
    let attack = c(Rank::Six, Suit::Spades);
    let extra = c(Rank::Six, Suit::Hearts);
    game.players[0].hand = vec![attack, c(Rank::Nine, Suit::Spades)];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades), c(Rank::Seven, Suit::Hearts)];
    game.players[2].hand = vec![extra];
    game = game.apply(cmd("a", Action::Attack(attack))).unwrap().state;
    game = game
        .apply(cmd(
            "b",
            Action::Defend {
                card: c(Rank::Seven, Suit::Spades),
                target: None,
            },
        ))
        .unwrap()
        .state;
    game = game.apply(cmd("a", Action::Pass)).unwrap().state;
    game = game.apply(cmd("c", Action::Attack(extra))).unwrap().state;
    game = game
        .apply(cmd(
            "b",
            Action::Defend {
                card: c(Rank::Seven, Suit::Hearts),
                target: None,
            },
        ))
        .unwrap()
        .state;
    assert_eq!(game.turn_id.as_deref(), Some("c"));
    assert_eq!(
        game.apply(cmd("a", Action::Pass)),
        Err(RuleError::WrongTurn)
    );
    game = game.apply(cmd("c", Action::Pass)).unwrap().state;
    assert!(game.table.is_empty());
}
