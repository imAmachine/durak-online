use std::collections::BTreeSet;

pub const MAX_TABLE_CARDS: usize = 6;
pub const HAND_LIMIT: usize = 6;
pub const MIN_PLAYERS: usize = 2;
pub const MAX_PLAYERS: usize = 6;

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum Suit {
    Spades,
    Hearts,
    Diamonds,
    Clubs,
}

impl Suit {
    pub const ALL: [Suit; 4] = [Suit::Spades, Suit::Hearts, Suit::Diamonds, Suit::Clubs];
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub enum Rank {
    Two,
    Three,
    Four,
    Five,
    Six,
    Seven,
    Eight,
    Nine,
    Ten,
    Jack,
    Queen,
    King,
    Ace,
}

impl Rank {
    pub const ALL_52: [Rank; 13] = [
        Rank::Two,
        Rank::Three,
        Rank::Four,
        Rank::Five,
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];
    pub const ALL_36: [Rank; 9] = [
        Rank::Six,
        Rank::Seven,
        Rank::Eight,
        Rank::Nine,
        Rank::Ten,
        Rank::Jack,
        Rank::Queen,
        Rank::King,
        Rank::Ace,
    ];
    fn value(self) -> usize {
        match self {
            Rank::Two => 0,
            Rank::Three => 1,
            Rank::Four => 2,
            Rank::Five => 3,
            Rank::Six => 4,
            Rank::Seven => 5,
            Rank::Eight => 6,
            Rank::Nine => 7,
            Rank::Ten => 8,
            Rank::Jack => 9,
            Rank::Queen => 10,
            Rank::King => 11,
            Rank::Ace => 12,
        }
    }
}

#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct Card {
    pub rank: Rank,
    pub suit: Suit,
}

impl Card {
    pub fn new(rank: Rank, suit: Suit) -> Self {
        Self { rank, suit }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DeckSize {
    ThirtySix,
    FiftyTwo,
}

impl DeckSize {
    fn ranks(self) -> &'static [Rank] {
        match self {
            Self::ThirtySix => &Rank::ALL_36,
            Self::FiftyTwo => &Rank::ALL_52,
        }
    }
    pub fn card_count(self) -> usize {
        self.ranks().len() * 4
    }
}

pub fn build_deck(size: DeckSize) -> Vec<Card> {
    Suit::ALL
        .into_iter()
        .flat_map(|suit| {
            size.ranks()
                .iter()
                .copied()
                .map(move |rank| Card::new(rank, suit))
        })
        .collect()
}

pub trait RandomSource {
    fn next_usize(&mut self, upper: usize) -> usize;
}

pub struct Lcg(pub u64);
impl RandomSource for Lcg {
    fn next_usize(&mut self, upper: usize) -> usize {
        self.0 = self.0.wrapping_mul(1664525).wrapping_add(1013904223);
        (self.0 as usize) % upper
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Mode {
    Classic,
    Transfer,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Status {
    Waiting,
    Playing,
    Finished,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Player {
    pub id: String,
    pub hand: Vec<Card>,
    pub finished: bool,
    pub left: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TableRow {
    pub attack: Card,
    pub defense: Option<Card>,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Finished {
    pub loser_id: Option<String>,
    pub draw: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DurakState {
    pub mode: Mode,
    pub deck_size: DeckSize,
    pub status: Status,
    pub players: Vec<Player>,
    pub deck: Vec<Card>,
    pub trump: Option<Suit>,
    pub trump_card: Option<Card>,
    pub table: Vec<TableRow>,
    pub discard: Vec<Card>,
    pub attacker_id: Option<String>,
    pub defender_id: Option<String>,
    pub turn_id: Option<String>,
    pub trick_attacker_id: Option<String>,
    pub passed_attackers: Vec<String>,
    pub take_pending: bool,
    pub finished: Option<Finished>,
}

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
    pub fn apply(&self, command: Command) -> Result<Transition, RuleError> {
        let mut next = self.clone();
        next.apply_mut(command).map(|_| next.transition("action"))
    }
    fn apply_mut(&mut self, command: Command) -> Result<(), RuleError> {
        if self.status != Status::Playing {
            return Err(RuleError::NotPlaying);
        }
        let actor = self
            .index(&command.player_id)
            .ok_or(RuleError::UnknownPlayer)?;
        if self.players[actor].left || self.players[actor].finished {
            return Err(RuleError::InactivePlayer);
        }
        match command.action {
            Action::Attack(card) => self.attack(actor, card),
            Action::Defend { card, target } => self.defend(actor, card, target),
            Action::Transfer(card) => self.transfer(actor, card),
            Action::Take => self.take(actor),
            Action::Pass => self.pass(actor),
        }
    }
    fn attack(&mut self, actor: usize, card: Card) -> Result<(), RuleError> {
        if Some(&self.players[actor].id) != self.turn_id.as_ref()
            || Some(&self.players[actor].id) == self.defender_id.as_ref()
        {
            return Err(RuleError::WrongTurn);
        }
        self.capacity()?;
        if self.table.len() >= MAX_TABLE_CARDS {
            return Err(RuleError::TableFull);
        }
        if !self.table.is_empty() && !self.table_ranks().contains(&card.rank) {
            return Err(RuleError::RankMismatch);
        }
        self.remove_card(actor, card)?;
        self.table.push(TableRow {
            attack: card,
            defense: None,
        });
        if self.take_pending {
            self.turn_id = Some(self.players[actor].id.clone());
        } else {
            self.update_turn();
        }
        Ok(())
    }
    fn defend(&mut self, actor: usize, card: Card, target: Option<Card>) -> Result<(), RuleError> {
        if Some(&self.players[actor].id) != self.defender_id.as_ref() {
            return Err(RuleError::WrongTurn);
        }
        if self.take_pending {
            return Err(RuleError::AlreadyTaking);
        }
        let row = self
            .table
            .iter()
            .position(|r| r.defense.is_none() && target.map_or(true, |c| c == r.attack))
            .ok_or(RuleError::NoUnbeatenRow)?;
        if !can_beat(
            self.table[row].attack,
            card,
            self.trump.ok_or(RuleError::NoTrump)?,
        ) {
            return Err(RuleError::CannotBeat);
        }
        self.remove_card(actor, card)?;
        self.table[row].defense = Some(card);
        self.update_turn();
        Ok(())
    }
    fn transfer(&mut self, actor: usize, card: Card) -> Result<(), RuleError> {
        if self.mode != Mode::Transfer {
            return Err(RuleError::TransferDisabled);
        }
        if Some(&self.players[actor].id) != self.defender_id.as_ref()
            || self.take_pending
            || self.table.is_empty()
            || self.table.iter().any(|r| r.defense.is_some())
        {
            return Err(RuleError::TransferUnavailable);
        }
        let rank = self.table[0].attack.rank;
        if self.table.iter().any(|r| r.attack.rank != rank) || card.rank != rank {
            return Err(RuleError::RankMismatch);
        }
        let next = self
            .next_active_id(actor)
            .ok_or(RuleError::NoNextDefender)?;
        let ni = self.index(&next).unwrap();
        if self.players[ni].hand.len() < self.table.len() + 1 || self.table.len() >= MAX_TABLE_CARDS
        {
            return Err(RuleError::InsufficientCapacity);
        }
        self.remove_card(actor, card)?;
        self.table.push(TableRow {
            attack: card,
            defense: None,
        });
        self.attacker_id = Some(self.players[actor].id.clone());
        self.defender_id = Some(next);
        self.passed_attackers.clear();
        self.update_turn();
        Ok(())
    }
    fn take(&mut self, actor: usize) -> Result<(), RuleError> {
        if Some(&self.players[actor].id) != self.defender_id.as_ref()
            || self.table.is_empty()
            || self.take_pending
            || !self.table.iter().any(|r| r.defense.is_none())
        {
            return Err(RuleError::TakeUnavailable);
        }
        self.take_pending = true;
        self.update_turn();
        Ok(())
    }
    fn pass(&mut self, actor: usize) -> Result<(), RuleError> {
        if Some(&self.players[actor].id) != self.turn_id.as_ref()
            || Some(&self.players[actor].id) == self.defender_id.as_ref()
            || self.table.is_empty()
        {
            return Err(RuleError::WrongTurn);
        }
        if !self.take_pending && self.table.iter().any(|r| r.defense.is_none()) {
            return Err(RuleError::UnbeatenCards);
        }
        self.passed_attackers.push(self.players[actor].id.clone());
        let throwing = self.take_pending || self.table.iter().all(|r| r.defense.is_some());
        if throwing {
            if self.take_pending {
                if let Some(next) = self.next_thrower_except(Some(self.players[actor].id.as_str()))
                {
                    self.turn_id = Some(next);
                    return Ok(());
                }
            } else if let Some(next) = self.next_thrower() {
                self.turn_id = Some(next);
                return Ok(());
            }
        }
        let defender = self
            .index_by_id(self.defender_id.as_deref())
            .ok_or(RuleError::UnknownPlayer)?;
        let taking = self.take_pending;
        if self.take_pending {
            let cards: Vec<Card> = self
                .table
                .iter()
                .flat_map(|r| [Some(r.attack), r.defense])
                .flatten()
                .collect();
            self.players[defender].hand.extend(cards);
            self.table.clear();
            self.take_pending = false;
        } else {
            if self.table.iter().any(|r| r.defense.is_none()) {
                return Err(RuleError::UnbeatenCards);
            }
            self.discard.extend(
                self.table
                    .iter()
                    .flat_map(|r| [Some(r.attack), r.defense])
                    .flatten(),
            );
            self.table.clear();
        }
        self.refill();
        let new_attacker = if taking {
            self.next_active_id(defender)
                .ok_or(RuleError::NoNextDefender)?
        } else {
            self.players[defender].id.clone()
        };
        self.attacker_id = Some(new_attacker.clone());
        self.defender_id = self.next_active_id(self.index(&new_attacker).unwrap());
        self.trick_attacker_id = self.attacker_id.clone();
        self.passed_attackers.clear();
        self.update_turn();
        self.settle();
        Ok(())
    }
    fn refill(&mut self) {
        let attacker = self
            .index_by_id(self.trick_attacker_id.as_deref())
            .unwrap_or(0);
        let defender = self.index_by_id(self.defender_id.as_deref());
        let mut order = Vec::new();
        let mut current = attacker;
        for _ in 0..self.players.len() {
            if Some(current) != defender
                && !self.players[current].finished
                && !self.players[current].left
            {
                order.push(current);
            }
            current = (current + 1) % self.players.len();
        }
        if let Some(d) = defender {
            if !self.players[d].finished && !self.players[d].left {
                order.push(d);
            }
        }
        for i in order {
            while self.players[i].hand.len() < HAND_LIMIT {
                match self.deck.pop() {
                    Some(card) => self.players[i].hand.push(card),
                    None => break,
                }
            }
        }
        if self.deck.is_empty() {
            self.trump_card = None;
        }
    }
    fn settle(&mut self) {
        if self.deck.is_empty() {
            for p in &mut self.players {
                if p.hand.is_empty() {
                    p.finished = true;
                }
            }
        }
        let active: Vec<_> = self
            .players
            .iter()
            .filter(|p| !p.finished && !p.left)
            .collect();
        if active.len() <= 1 {
            self.status = Status::Finished;
            self.finished = Some(Finished {
                loser_id: active.first().map(|p| p.id.clone()),
                draw: active.is_empty(),
            });
            self.attacker_id = None;
            self.defender_id = None;
            self.turn_id = None;
        } else {
            self.normalize_roles();
        }
    }
    fn normalize_roles(&mut self) {
        let active = |p: &Player| !p.finished && !p.left;
        let attacker = self
            .attacker_id
            .as_deref()
            .and_then(|id| self.index(id))
            .filter(|&i| active(&self.players[i]))
            .or_else(|| {
                self.attacker_id
                    .as_deref()
                    .and_then(|id| self.index(id))
                    .and_then(|i| self.next_active_index(i))
            })
            .or_else(|| self.players.iter().position(active));
        let Some(attacker) = attacker else {
            self.attacker_id = None;
            self.defender_id = None;
            self.turn_id = None;
            return;
        };
        self.attacker_id = Some(self.players[attacker].id.clone());
        let defender_valid = self
            .defender_id
            .as_deref()
            .and_then(|id| self.index(id))
            .filter(|&i| i != attacker && active(&self.players[i]))
            .is_some();
        if !defender_valid {
            self.defender_id = self.next_active_id(attacker);
        }
        let turn_valid = self
            .turn_id
            .as_deref()
            .and_then(|id| self.index(id))
            .is_some_and(|i| active(&self.players[i]));
        if !turn_valid {
            self.turn_id = self.attacker_id.clone();
        }
        if self.table.is_empty() && self.turn_id.as_deref() != self.attacker_id.as_deref() {
            self.turn_id = self.attacker_id.clone();
        }
    }
    fn capacity(&self) -> Result<(), RuleError> {
        let defender = self
            .index_by_id(self.defender_id.as_deref())
            .ok_or(RuleError::UnknownPlayer)?;
        if self.players[defender].hand.len()
            <= self.table.iter().filter(|r| r.defense.is_none()).count()
        {
            return Err(RuleError::InsufficientCapacity);
        }
        Ok(())
    }
    fn next_thrower_except(&self, skip: Option<&str>) -> Option<String> {
        let start = self.index_by_id(self.attacker_id.as_deref())?;
        (0..self.players.len())
            .map(|n| &self.players[(start + n) % self.players.len()])
            .find(|p| {
                !p.finished
                    && !p.left
                    && Some(&p.id) != self.defender_id.as_ref()
                    && !self.passed_attackers.contains(&p.id)
                    && skip != Some(p.id.as_str())
            })
            .map(|p| p.id.clone())
    }
    fn next_thrower(&self) -> Option<String> {
        self.next_thrower_except(None)
    }
    fn update_turn(&mut self) {
        self.turn_id = if self.table.is_empty() {
            self.attacker_id.clone()
        } else if self.take_pending || self.table.iter().all(|r| r.defense.is_some()) {
            self.next_thrower().or_else(|| self.attacker_id.clone())
        } else {
            self.defender_id.clone()
        };
    }
    fn table_ranks(&self) -> BTreeSet<Rank> {
        self.table
            .iter()
            .flat_map(|r| {
                [Some(r.attack.rank), r.defense.map(|c| c.rank)]
                    .into_iter()
                    .flatten()
            })
            .collect()
    }
    fn remove_card(&mut self, player: usize, card: Card) -> Result<(), RuleError> {
        let pos = self.players[player]
            .hand
            .iter()
            .position(|c| *c == card)
            .ok_or(RuleError::CardNotInHand)?;
        self.players[player].hand.remove(pos);
        Ok(())
    }
    fn index(&self, id: &str) -> Option<usize> {
        self.players.iter().position(|p| p.id == id)
    }
    fn index_by_id(&self, id: Option<&str>) -> Option<usize> {
        id.and_then(|v| self.index(v))
    }
    fn next_active_index(&self, from: usize) -> Option<usize> {
        (1..=self.players.len())
            .map(|n| (from + n) % self.players.len())
            .find(|&i| !self.players[i].finished && !self.players[i].left)
    }
    fn next_active_id(&self, from: usize) -> Option<String> {
        (1..=self.players.len())
            .map(|n| (from + n) % self.players.len())
            .map(|i| &self.players[i])
            .find(|p| !p.finished && !p.left)
            .map(|p| p.id.clone())
    }
    fn transition(&self, kind: &str) -> Transition {
        Transition {
            state: self.clone(),
            event: Event {
                kind: kind.to_owned(),
            },
            settlement: if self.status == Status::Finished {
                Some(SettlementIntent {
                    loser_id: self.finished.as_ref().and_then(|f| f.loser_id.clone()),
                })
            } else {
                None
            },
        }
    }
}

pub fn can_beat(attack: Card, defense: Card, trump: Suit) -> bool {
    if attack.suit == defense.suit {
        defense.rank.value() > attack.rank.value()
    } else {
        defense.suit == trump && attack.suit != trump
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    Attack(Card),
    Defend { card: Card, target: Option<Card> },
    Transfer(Card),
    Take,
    Pass,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Command {
    pub player_id: String,
    pub action: Action,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Event {
    pub kind: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettlementIntent {
    pub loser_id: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    pub state: DurakState,
    pub event: Event,
    pub settlement: Option<SettlementIntent>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuleError {
    AlreadyPlaying,
    NeedTwoPlayers,
    TooManyPlayers,
    EmptyPlayerId,
    DuplicatePlayerId,
    NotPlaying,
    UnknownPlayer,
    InactivePlayer,
    WrongTurn,
    CardNotInHand,
    TableFull,
    RankMismatch,
    NoUnbeatenRow,
    CannotBeat,
    NoTrump,
    AlreadyTaking,
    TransferDisabled,
    TransferUnavailable,
    NoNextDefender,
    InsufficientCapacity,
    TakeUnavailable,
    UnbeatenCards,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn c(rank: Rank, suit: Suit) -> Card {
        Card::new(rank, suit)
    }
    fn cmd(id: &str, action: Action) -> Command {
        Command {
            player_id: id.into(),
            action,
        }
    }
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
                    t.state.deck.len()
                        + t.state.players.iter().map(|p| p.hand.len()).sum::<usize>(),
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
    }
}
