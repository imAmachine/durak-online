use crate::{Action, Card, DeckSize, DurakState, Lcg, Mode, Rank, RuleError, Status, Suit};

use super::{c, cmd};

#[test]
fn invalid_command_is_atomic() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    let mut rng = Lcg(1);
    game.start(&mut rng).unwrap();
    let before = game.clone();
    let card = Card::new(Rank::Two, Suit::Spades);
    assert!(game.apply(cmd("b", Action::Attack(card))).is_err());
    assert_eq!(game, before);
}
#[test]
fn attack_defend_and_pass_rotate_and_refill() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Clubs);
    game.deck = vec![c(Rank::Ace, Suit::Diamonds), c(Rank::King, Suit::Diamonds)];
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.trick_attacker_id = Some("a".into());
    game.players[0].hand = vec![c(Rank::Six, Suit::Spades)];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades)];
    game.players[2].hand = vec![];
    game.players[2].finished = true;
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
    game = game.apply(cmd("a", Action::Pass)).unwrap().state;
    assert_eq!(game.players[0].hand.len(), 2);
    assert_eq!(game.status, Status::Finished);
    assert_eq!(game.attacker_id, None);
    assert_eq!(game.defender_id, None);
}
#[test]
fn take_requires_an_unbeaten_attack_and_is_atomic() {
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
    let attack = c(Rank::Six, Suit::Spades);
    let defense = c(Rank::Seven, Suit::Spades);
    game.players[0].hand.push(attack);
    game.players[1].hand.push(defense);
    assert_eq!(
        game.apply(cmd("b", Action::Take)),
        Err(RuleError::TakeUnavailable)
    );
    assert!(game.table.is_empty());
    game = game.apply(cmd("a", Action::Attack(attack))).unwrap().state;
    assert_eq!(
        game.apply(cmd("a", Action::Take)),
        Err(RuleError::TakeUnavailable)
    );
    assert!(
        game.apply(cmd("b", Action::Take))
            .unwrap()
            .state
            .take_pending
    );
    game = game
        .apply(cmd(
            "b",
            Action::Defend {
                card: defense,
                target: None,
            },
        ))
        .unwrap()
        .state;
    assert_eq!(
        game.apply(cmd("b", Action::Take)),
        Err(RuleError::TakeUnavailable)
    );
}
#[test]
fn take_allows_attackers_to_add_then_collects() {
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
    game.players[0].hand = vec![c(Rank::Six, Suit::Spades), c(Rank::Six, Suit::Hearts)];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades), c(Rank::Eight, Suit::Hearts)];
    game = game
        .apply(cmd("a", Action::Attack(c(Rank::Six, Suit::Spades))))
        .unwrap()
        .state;
    game = game.apply(cmd("b", Action::Take)).unwrap().state;
    game = game
        .apply(cmd("a", Action::Attack(c(Rank::Six, Suit::Hearts))))
        .unwrap()
        .state;
    game = game.apply(cmd("a", Action::Pass)).unwrap().state;
    assert!(game.players[1].hand.iter().any(|x| x.rank == Rank::Six));
    assert!(game.table.is_empty());
}
#[test]
fn transfer_requires_same_rank_and_capacity() {
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
    game.players[0].hand = vec![c(Rank::Six, Suit::Spades)];
    game.players[1].hand = vec![c(Rank::Six, Suit::Hearts)];
    game.players[2].hand = vec![c(Rank::Nine, Suit::Spades), c(Rank::Ten, Suit::Spades)];
    game = game
        .apply(cmd("a", Action::Attack(c(Rank::Six, Suit::Spades))))
        .unwrap()
        .state;
    let t = game
        .apply(cmd("b", Action::Transfer(c(Rank::Six, Suit::Hearts))))
        .unwrap();
    assert_eq!(t.state.defender_id.as_deref(), Some("c"));
}

#[test]
fn settle_reassigns_roles_when_attacker_finishes() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.deck.clear();
    game.trump = Some(Suit::Clubs);
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.trick_attacker_id = Some("a".into());
    game.players[0].hand = vec![c(Rank::Six, Suit::Spades)];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades), c(Rank::Ten, Suit::Hearts)];
    game.players[2].hand = vec![c(Rank::Eight, Suit::Spades), c(Rank::Nine, Suit::Spades)];
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
    game = game.apply(cmd("a", Action::Pass)).unwrap().state;
    assert_eq!(game.turn_id.as_deref(), Some("c"));
    game = game.apply(cmd("c", Action::Pass)).unwrap().state;
    assert_eq!(game.status, Status::Playing);
    assert_eq!(game.attacker_id.as_deref(), Some("b"));
    assert_eq!(game.defender_id.as_deref(), Some("c"));
    assert_eq!(game.turn_id.as_deref(), Some("b"));
}
