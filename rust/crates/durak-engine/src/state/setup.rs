use std::collections::BTreeSet;

use crate::cards::{build_deck, Card, DeckSize, RandomSource};

use super::{
    DurakState, Mode, Player, RuleError, Status, Transition, HAND_LIMIT, MAX_PLAYERS, MIN_PLAYERS,
};

impl DurakState {
    pub fn new(ids: Vec<String>, mode: Mode, deck_size: DeckSize) -> Self {
        Self {
            mode,
            deck_size,
            status: Status::Waiting,
            players: ids
                .into_iter()
                .map(|id| Player {
                    id,
                    hand: Vec::new(),
                    finished: false,
                    left: false,
                })
                .collect(),
            deck: Vec::new(),
            trump: None,
            trump_card: None,
            table: Vec::new(),
            discard: Vec::new(),
            attacker_id: None,
            defender_id: None,
            turn_id: None,
            trick_attacker_id: None,
            passed_attackers: Vec::new(),
            take_pending: false,
            finished: None,
        }
    }
    pub fn start<R: RandomSource>(&mut self, rng: &mut R) -> Result<Transition, RuleError> {
        if self.status == Status::Playing {
            return Err(RuleError::AlreadyPlaying);
        }
        if self.players.len() < MIN_PLAYERS {
            return Err(RuleError::NeedTwoPlayers);
        }
        if self.players.len() > MAX_PLAYERS {
            return Err(RuleError::TooManyPlayers);
        }
        if self.players.iter().any(|p| p.id.is_empty()) {
            return Err(RuleError::EmptyPlayerId);
        }
        let mut ids = BTreeSet::new();
        if self.players.iter().any(|p| !ids.insert(p.id.as_str())) {
            return Err(RuleError::DuplicatePlayerId);
        }
        let mut deck = build_deck(self.deck_size);
        for i in (1..deck.len()).rev() {
            let j = rng.next_usize(i + 1);
            deck.swap(i, j);
        }
        self.deck = deck;
        self.trump_card = self.deck.first().copied();
        self.trump = self.trump_card.map(|c| c.suit);
        self.table.clear();
        self.discard.clear();
        self.finished = None;
        self.passed_attackers.clear();
        self.take_pending = false;
        for p in &mut self.players {
            p.hand.clear();
            p.finished = false;
            p.left = false;
        }
        for _ in 0..HAND_LIMIT {
            for p in &mut self.players {
                if let Some(card) = self.deck.pop() {
                    p.hand.push(card);
                }
            }
        }
        if self.deck.is_empty() {
            self.trump_card = None;
        }
        let trump = self.trump.ok_or(RuleError::NoTrump)?;
        let candidates: Vec<(usize, Card)> = self
            .players
            .iter()
            .enumerate()
            .flat_map(|(i, p)| p.hand.iter().copied().map(move |c| (i, c)))
            .collect();
        let trumps: Vec<_> = candidates
            .iter()
            .copied()
            .filter(|(_, c)| c.suit == trump)
            .collect();
        let pool = if trumps.is_empty() {
            candidates
        } else {
            trumps
        };
        let (attacker, _) = pool
            .into_iter()
            .min_by_key(|(_, c)| c.rank.value())
            .unwrap();
        self.status = Status::Playing;
        self.attacker_id = Some(self.players[attacker].id.clone());
        self.trick_attacker_id = self.attacker_id.clone();
        self.defender_id = self.next_active_id(attacker);
        self.turn_id = self.attacker_id.clone();
        Ok(self.transition("start"))
    }
}
