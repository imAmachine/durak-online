use std::collections::BTreeSet;

use crate::cards::{can_beat, Card, Rank};

use super::{DurakState, Mode, Status, TableRow, MAX_TABLE_CARDS};

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

impl DurakState {
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
            .position(|r| r.defense.is_none() && target.is_none_or(|c| c == r.attack))
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
}
