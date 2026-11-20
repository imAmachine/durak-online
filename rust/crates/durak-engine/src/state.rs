use crate::cards::{Card, DeckSize, Suit};

mod actions;
mod round;
mod setup;

pub use actions::{Action, Command, Event, RuleError, SettlementIntent, Transition};

pub const MAX_TABLE_CARDS: usize = 6;
pub const HAND_LIMIT: usize = 6;
pub const MIN_PLAYERS: usize = 2;
pub const MAX_PLAYERS: usize = 6;

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
