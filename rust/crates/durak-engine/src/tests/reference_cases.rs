use crate::{Action, DeckSize, DurakState, Mode, Rank, RuleError, Status, Suit};

use super::{c, cmd};

#[test]
fn attack_capacity_counts_only_unbeaten_rows() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Clubs);
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.players[0].hand = vec![
        c(Rank::Six, Suit::Spades),
        c(Rank::Seven, Suit::Hearts),
        c(Rank::Six, Suit::Hearts),
    ];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades), c(Rank::Eight, Suit::Clubs)];
    game = game
        .apply(cmd("a", Action::Attack(c(Rank::Six, Suit::Spades))))
        .unwrap()
        .state;
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
    game = game
        .apply(cmd("a", Action::Attack(c(Rank::Seven, Suit::Hearts))))
        .unwrap()
        .state;
    game = game.apply(cmd("b", Action::Take)).unwrap().state;
    assert_eq!(
        game.apply(cmd("a", Action::Attack(c(Rank::Six, Suit::Hearts)))),
        Err(RuleError::InsufficientCapacity)
    );
}

#[test]
fn actions_reject_wrong_roles_and_unfinished_table() {
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
    game.players[0].hand = vec![c(Rank::Six, Suit::Spades)];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades)];
    game.players[2].hand = vec![c(Rank::Six, Suit::Hearts)];
    assert_eq!(
        game.apply(cmd("c", Action::Attack(c(Rank::Six, Suit::Hearts)))),
        Err(RuleError::WrongTurn)
    );
    game = game
        .apply(cmd("a", Action::Attack(c(Rank::Six, Suit::Spades))))
        .unwrap()
        .state;
    assert_eq!(
        game.apply(cmd("b", Action::Pass)),
        Err(RuleError::WrongTurn)
    );
    assert_eq!(
        game.apply(cmd("c", Action::Pass)),
        Err(RuleError::WrongTurn)
    );
    assert_eq!(
        game.apply(cmd("a", Action::Pass)),
        Err(RuleError::WrongTurn)
    );
}

#[test]
fn transfer_is_rejected_after_defense_take_and_in_classic_mode() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into()],
        Mode::Transfer,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Clubs);
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    let attack = c(Rank::Six, Suit::Spades);
    let transfer = c(Rank::Six, Suit::Hearts);
    game.players[0].hand = vec![attack];
    game.players[1].hand = vec![transfer, c(Rank::Seven, Suit::Spades)];
    game.players[2].hand = vec![c(Rank::Eight, Suit::Spades), c(Rank::Nine, Suit::Spades)];
    game = game.apply(cmd("a", Action::Attack(attack))).unwrap().state;
    let taking = game.apply(cmd("b", Action::Take)).unwrap().state;
    assert_eq!(
        taking.apply(cmd("b", Action::Transfer(transfer))),
        Err(RuleError::TransferUnavailable)
    );
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
    assert_eq!(
        game.apply(cmd("b", Action::Transfer(transfer))),
        Err(RuleError::TransferUnavailable)
    );
    let mut classic = game.clone();
    classic.mode = Mode::Classic;
    assert_eq!(
        classic.apply(cmd("b", Action::Transfer(transfer))),
        Err(RuleError::TransferDisabled)
    );
}

#[test]
fn empty_deck_finishes_after_take_or_full_defense() {
    let attack = c(Rank::Six, Suit::Spades);
    let defense = c(Rank::Seven, Suit::Spades);
    let mut defended = DurakState::new(
        vec!["a".into(), "b".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    defended.status = Status::Playing;
    defended.trump = Some(Suit::Clubs);
    defended.attacker_id = Some("a".into());
    defended.defender_id = Some("b".into());
    defended.turn_id = Some("a".into());
    defended.players[0].hand = vec![attack];
    defended.players[1].hand = vec![defense];
    defended = defended
        .apply(cmd("a", Action::Attack(attack)))
        .unwrap()
        .state;
    defended = defended
        .apply(cmd(
            "b",
            Action::Defend {
                card: defense,
                target: None,
            },
        ))
        .unwrap()
        .state;
    defended = defended.apply(cmd("a", Action::Pass)).unwrap().state;
    assert_eq!(defended.status, Status::Finished);
    assert!(defended.finished.as_ref().unwrap().draw);

    let mut taken = DurakState::new(
        vec!["a".into(), "b".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    taken.status = Status::Playing;
    taken.trump = Some(Suit::Clubs);
    taken.attacker_id = Some("a".into());
    taken.defender_id = Some("b".into());
    taken.turn_id = Some("a".into());
    taken.players[0].hand = vec![attack];
    taken.players[1].hand = vec![defense];
    taken = taken.apply(cmd("a", Action::Attack(attack))).unwrap().state;
    taken = taken.apply(cmd("b", Action::Take)).unwrap().state;
    taken = taken.apply(cmd("a", Action::Pass)).unwrap().state;
    assert_eq!(taken.status, Status::Finished);
    assert_eq!(
        taken.finished.as_ref().unwrap().loser_id.as_deref(),
        Some("b")
    );
}
