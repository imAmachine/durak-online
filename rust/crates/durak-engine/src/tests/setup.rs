use std::collections::BTreeSet;

use crate::{can_beat, DeckSize, DurakState, Lcg, Mode, Rank, RuleError, Status, Suit};

use super::c;

#[test]
fn builds_and_deals_all_sizes_without_duplicates() {
    for size in [DeckSize::ThirtySix, DeckSize::FiftyTwo] {
        for count in 2..=6 {
            let mut game = DurakState::new(
                (0..count).map(|i| format!("p{i}")).collect(),
                Mode::Classic,
                size,
            );
            let mut rng = Lcg(7);
            let t = game.start(&mut rng).unwrap();
            assert_eq!(
                t.state.deck.len() + t.state.players.iter().map(|p| p.hand.len()).sum::<usize>(),
                size.card_count()
            );
            let mut cards = t.state.deck.clone();
            cards.extend(t.state.players.iter().flat_map(|p| p.hand.iter().copied()));
            assert_eq!(cards.len(), cards.iter().collect::<BTreeSet<_>>().len());
            assert_eq!(
                t.state.players.iter().map(|p| p.hand.len()).sum::<usize>(),
                count * 6
            );
        }
    }
}

#[test]
fn start_rejects_player_counts_outside_two_to_six() {
    for count in [0, 1, 7] {
        let mut game = DurakState::new(
            (0..count).map(|i| format!("p{i}")).collect(),
            Mode::Classic,
            DeckSize::ThirtySix,
        );
        let result = game.start(&mut Lcg(1));
        assert!(
            matches!(result, Err(RuleError::NeedTwoPlayers) if count < 2)
                || matches!(result, Err(RuleError::TooManyPlayers) if count > 6)
        );
    }
    for count in [2, 6] {
        let mut game = DurakState::new(
            (0..count).map(|i| format!("p{i}")).collect(),
            Mode::Classic,
            DeckSize::ThirtySix,
        );
        assert!(game.start(&mut Lcg(1)).is_ok());
    }
}
#[test]
fn start_rejects_empty_and_duplicate_player_ids_atomically() {
    for (ids, expected) in [
        (vec!["a", ""], RuleError::EmptyPlayerId),
        (vec!["a", "a"], RuleError::DuplicatePlayerId),
        (vec!["", ""], RuleError::EmptyPlayerId),
        (vec!["a", "b", "a"], RuleError::DuplicatePlayerId),
    ] {
        let mut game = DurakState::new(
            ids.into_iter().map(String::from).collect(),
            Mode::Classic,
            DeckSize::ThirtySix,
        );
        let before = game.clone();
        let result = game.start(&mut Lcg(1));
        assert_eq!(result, Err(expected));
        assert_eq!(game, before);
        assert_eq!(game.status, Status::Waiting);
        assert!(game.deck.is_empty());
    }
}
#[test]
fn trump_card_is_hidden_when_deck_is_empty_after_deal() {
    let mut game = DurakState::new(
        vec![
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
            "e".into(),
            "f".into(),
        ],
        Mode::Classic,
        DeckSize::ThirtySix,
    );
    let transition = game.start(&mut Lcg(1)).unwrap();
    assert!(transition.state.deck.is_empty());
    assert_eq!(transition.state.trump_card, None);
    assert!(transition.state.trump.is_some());
}

#[test]
fn opening_attacker_holds_the_lowest_trump_or_lowest_card() {
    struct NoShuffle;
    impl crate::RandomSource for NoShuffle {
        fn next_usize(&mut self, upper: usize) -> usize {
            upper - 1
        }
    }
    let mut saw_no_trump = false;
    let mut saw_trump = false;
    for size in [DeckSize::ThirtySix, DeckSize::FiftyTwo] {
        for count in 2..=6 {
            for seed in 0..20 {
                let mut game = DurakState::new(
                    (0..count).map(|index| format!("p{index}")).collect(),
                    Mode::Classic,
                    size,
                );
                let state = if seed == 0 {
                    game.start(&mut NoShuffle).unwrap().state
                } else {
                    game.start(&mut Lcg(seed)).unwrap().state
                };
                let trump = state.trump.unwrap();
                let held: Vec<_> = state
                    .players
                    .iter()
                    .flat_map(|player| player.hand.iter().map(move |card| (player, card)))
                    .collect();
                let trumps: Vec<_> = held
                    .iter()
                    .copied()
                    .filter(|(_, card)| card.suit == trump)
                    .collect();
                saw_no_trump |= trumps.is_empty();
                saw_trump |= !trumps.is_empty();
                let pool = if trumps.is_empty() { held } else { trumps };
                let (player, _) = pool.into_iter().min_by_key(|(_, card)| card.rank).unwrap();
                assert_eq!(state.attacker_id.as_deref(), Some(player.id.as_str()));
            }
        }
    }
    assert!(saw_trump && saw_no_trump);
}

#[test]
fn trump_beats_non_trump_only() {
    assert!(can_beat(
        c(Rank::Ace, Suit::Spades),
        c(Rank::Six, Suit::Clubs),
        Suit::Clubs
    ));
    assert!(!can_beat(
        c(Rank::Six, Suit::Clubs),
        c(Rank::Ace, Suit::Spades),
        Suit::Clubs
    ));
    assert!(!can_beat(
        c(Rank::Ace, Suit::Spades),
        c(Rank::Six, Suit::Hearts),
        Suit::Clubs
    ));
    assert!(can_beat(
        c(Rank::Six, Suit::Spades),
        c(Rank::Seven, Suit::Spades),
        Suit::Clubs
    ));
    assert!(!can_beat(
        c(Rank::Seven, Suit::Spades),
        c(Rank::Six, Suit::Spades),
        Suit::Clubs
    ));
}
