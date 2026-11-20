use durak_engine::{Card, Rank, Suit};
use serde_json::{json, Value};

pub(super) fn card(v: &Value) -> Option<Card> {
    let id = v.as_str().or_else(|| v.get("id").and_then(Value::as_str))?;
    let (pos, _) = id.char_indices().last()?;
    let (rank, suit) = id.split_at(pos);
    Some(Card::new(Rank::from_label(rank)?, Suit::from_label(suit)?))
}

pub(super) fn card_label(c: Card) -> String {
    format!("{}{}", c.rank.label(), c.suit.label())
}

pub(super) fn card_json(c: Card) -> Value {
    json!({"id":card_label(c),"rank":c.rank.label(),"suit":c.suit.label()})
}
