use crate::{Action, DeckSize, DurakState, Mode, Rank, Status, Suit};

use super::{c, cmd};

#[test]
fn refill_deals_from_trick_attacker_around_to_defender() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into(), "c".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Diamonds);
    game.trump_card = Some(c(Rank::Six, Suit::Diamonds));
    game.deck = vec![
        c(Rank::Six, Suit::Diamonds),
        c(Rank::Seven, Suit::Diamonds),
        c(Rank::Eight, Suit::Diamonds),
        c(Rank::Nine, Suit::Diamonds),
        c(Rank::Ten, Suit::Diamonds),
        c(Rank::Jack, Suit::Diamonds),
    ];
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.trick_attacker_id = Some("a".into());
    game.players[0].hand = [Rank::Six, Rank::Eight, Rank::Nine, Rank::Ten, Rank::Jack]
        .into_iter()
        .map(|r| c(r, Suit::Spades))
        .collect();
    game.players[1].hand = vec![
        c(Rank::Seven, Suit::Spades),
        c(Rank::Eight, Suit::Hearts),
        c(Rank::Nine, Suit::Hearts),
        c(Rank::Ten, Suit::Hearts),
        c(Rank::Jack, Suit::Hearts),
    ];
    game.players[2].hand = [Rank::Eight, Rank::Nine, Rank::Ten, Rank::Jack, Rank::Queen]
        .into_iter()
        .map(|r| c(r, Suit::Clubs))
        .collect();
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
    game = game.apply(cmd("c", Action::Pass)).unwrap().state;
    assert_eq!(game.players[0].hand.len(), 6);
    assert_eq!(game.players[1].hand.len(), 6);
    assert_eq!(game.players[2].hand.len(), 6);
    assert!(game.players[0]
        .hand
        .contains(&c(Rank::Jack, Suit::Diamonds)));
    assert!(game.players[0].hand.contains(&c(Rank::Ten, Suit::Diamonds)));
    assert!(game.players[2]
        .hand
        .contains(&c(Rank::Nine, Suit::Diamonds)));
    assert!(game.players[1]
        .hand
        .contains(&c(Rank::Eight, Suit::Diamonds)));
    assert!(game.players[1]
        .hand
        .contains(&c(Rank::Seven, Suit::Diamonds)));
    assert_eq!(game.deck, vec![c(Rank::Six, Suit::Diamonds)]);
    assert_eq!(game.trump_card, game.deck.first().copied());
}

#[test]
fn refill_hides_trump_card_when_the_last_deck_card_is_drawn() {
    let mut game = DurakState::new(
        vec!["a".into(), "b".into()],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    game.status = Status::Playing;
    game.trump = Some(Suit::Diamonds);
    game.trump_card = Some(c(Rank::Six, Suit::Diamonds));
    game.deck = vec![c(Rank::Six, Suit::Diamonds)];
    game.attacker_id = Some("a".into());
    game.defender_id = Some("b".into());
    game.turn_id = Some("a".into());
    game.trick_attacker_id = Some("a".into());
    game.players[0].hand = vec![c(Rank::Six, Suit::Spades)];
    game.players[1].hand = vec![c(Rank::Seven, Suit::Spades)];
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
    assert!(game.deck.is_empty());
    assert_eq!(game.trump_card, None);
}
