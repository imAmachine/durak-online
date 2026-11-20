use crate::{Action, Card, Command, Rank, Suit};

mod actions;
mod draw_order;
mod reference_cases;
mod setup;
mod throw_ins;

fn c(rank: Rank, suit: Suit) -> Card {
    Card::new(rank, suit)
}
fn cmd(id: &str, action: Action) -> Command {
    Command {
        player_id: id.into(),
        action,
    }
}
