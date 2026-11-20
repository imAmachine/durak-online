use serde_json::{json, Value};
use std::collections::HashSet;

const RANKS: [&str; 13] = [
    "2", "3", "4", "5", "6", "7", "8", "9", "10", "В", "Д", "К", "Т",
];
const SUITS: [&str; 4] = ["♠", "♥", "♦", "♣"];
const MAX_TABLE_CARDS: usize = 6;

pub fn value(rank: &str) -> Option<usize> {
    RANKS.iter().position(|r| *r == rank)
}

pub fn card_text(c: &Value) -> String {
    format!(
        "{}{}",
        c["rank"].as_str().unwrap_or(""),
        c["suit"].as_str().unwrap_or("")
    )
}

pub fn parse_card(id: &str) -> Option<Value> {
    let suit = id.chars().last()?.to_string();
    if !SUITS.contains(&suit.as_str()) {
        return None;
    }
    let rank = id[..id.len() - suit.len()].to_string();
    value(&rank)?;
    Some(json!({"id":id,"rank":rank,"suit":suit}))
}

pub fn beats(attack: &Value, defense: &Value, trump: &str) -> bool {
    if attack.is_null() || defense.is_null() {
        return false;
    }
    if attack["suit"] == defense["suit"] {
        return rank_of(defense) > rank_of(attack);
    }
    defense["suit"] == trump && attack["suit"] != trump
}

fn rank_of(c: &Value) -> usize {
    c["rank"].as_str().and_then(value).unwrap_or(0)
}

fn is_trump(c: &Value, st: &Value) -> bool {
    !st["trump"].is_null() && c["suit"] == st["trump"]
}

fn hand(st: &Value) -> Vec<Value> {
    st["hand"].as_array().cloned().unwrap_or_default()
}

fn table(st: &Value) -> Vec<Value> {
    st["table"].as_array().cloned().unwrap_or_default()
}

fn unbeaten(st: &Value) -> usize {
    table(st)
        .iter()
        .filter(|row| row["defense"].is_null())
        .count()
}

fn table_ranks(st: &Value) -> Vec<String> {
    let mut ranks: Vec<String> = Vec::new();
    for row in table(st) {
        if let Some(rank) = row["attack"]["rank"].as_str() {
            if !ranks.iter().any(|r| r == rank) {
                ranks.push(rank.to_string());
            }
        }
        if let Some(rank) = row["defense"]["rank"].as_str() {
            if !ranks.iter().any(|r| r == rank) {
                ranks.push(rank.to_string());
            }
        }
    }
    ranks
}

pub fn attach_options(st: &Value) -> Vec<Value> {
    if st["canAttack"] != true {
        return vec![];
    }
    let cards = hand(st);
    if table(st).is_empty() {
        return cards;
    }
    let ranks = table_ranks(st);
    cards
        .into_iter()
        .filter(|c| ranks.iter().any(|r| Some(r.as_str()) == c["rank"].as_str()))
        .collect()
}

pub fn defense_options(st: &Value) -> Vec<(Value, Value)> {
    if st["canDefend"] != true {
        return vec![];
    }
    let mut options = Vec::new();
    for row in table(st) {
        if !row["defense"].is_null() {
            continue;
        }
        let attack = &row["attack"];
        for c in hand(st) {
            if beats(attack, &c, st["trump"].as_str().unwrap_or("")) {
                options.push((row.clone(), c));
            }
        }
    }
    options.sort_by_key(|(_, c)| (is_trump(c, st), rank_of(c)));
    options
}

pub fn transfer_options(st: &Value) -> Vec<Value> {
    if st["canTransfer"] != true {
        return vec![];
    }
    let rank = match table_ranks(st).first() {
        Some(rank) => rank.clone(),
        None => return vec![],
    };
    hand(st)
        .into_iter()
        .filter(|c| c["rank"] == rank.as_str())
        .collect()
}

pub fn dump_options(st: &Value) -> Vec<Value> {
    if st["canAttack"] != true {
        return vec![];
    }
    let ranks = table_ranks(st);
    let mut cards: Vec<Value> = hand(st)
        .into_iter()
        .filter(|c| ranks.iter().any(|r| Some(r.as_str()) == c["rank"].as_str()))
        .filter(|c| !is_trump(c, st))
        .collect();
    cards.sort_by_key(rank_of);
    cards
}

pub fn attack_choices(st: &Value) -> Vec<Value> {
    let cards = attach_options(st);
    if cards.is_empty() || table(st).len() >= MAX_TABLE_CARDS {
        return vec![];
    }
    let mut plain: Vec<Value> = cards.iter().filter(|c| !is_trump(c, st)).cloned().collect();
    let mut trumps: Vec<Value> = cards.iter().filter(|c| is_trump(c, st)).cloned().collect();
    plain.sort_by_key(rank_of);
    trumps.sort_by_key(rank_of);
    if table(st).is_empty() {
        plain.extend(trumps);
        return plain;
    }
    if !plain.is_empty() {
        return plain;
    }
    let defender_cards = st["players"]
        .as_array()
        .and_then(|players| {
            players
                .iter()
                .find(|p| p["id"] == st["defenderId"])
                .and_then(|p| p["cards"].as_u64())
        })
        .unwrap_or(6);
    if st["deck"].as_u64().unwrap_or(1) == 0 || defender_cards <= 2 {
        return trumps;
    }
    vec![]
}

#[derive(Default)]
pub struct Ledger {
    room_id: Option<String>,
    seen: HashSet<String>,
    dead: HashSet<String>,
    pending: Vec<(Value, Option<Value>)>,
    last_seq: u64,
}

impl Ledger {
    fn reset(&mut self, room_id: Option<String>) {
        self.room_id = room_id;
        self.seen.clear();
        self.dead.clear();
        self.pending.clear();
        self.last_seq = 0;
    }

    pub fn observe(&mut self, st: &Value) {
        if !st.is_object() {
            return;
        }
        let room_id = st["room"]["id"].as_str().map(str::to_string);
        if room_id != self.room_id {
            self.reset(room_id);
        }
        for c in hand(st) {
            if let Some(id) = c["id"].as_str() {
                self.seen.insert(id.to_string());
            }
        }
        if let Some(id) = st["trumpCard"]["id"].as_str() {
            self.seen.insert(id.to_string());
        }
        for row in table(st) {
            if let Some(id) = row["attack"]["id"].as_str() {
                self.seen.insert(id.to_string());
            }
            if let Some(id) = row["defense"]["id"].as_str() {
                self.seen.insert(id.to_string());
            }
        }
        let mut events: Vec<Value> = st["log"].as_array().cloned().unwrap_or_default();
        events.sort_by_key(|e| e["seq"].as_u64().unwrap_or(0));
        for event in events {
            self.apply(&event);
        }
        if self.pending.is_empty() && !table(st).is_empty() {
            self.pending = table(st)
                .into_iter()
                .map(|row| {
                    let defense = if row["defense"].is_null() {
                        None
                    } else {
                        Some(row["defense"].clone())
                    };
                    (row["attack"].clone(), defense)
                })
                .collect();
        }
    }

    fn apply(&mut self, event: &Value) {
        let seq = match event["seq"].as_u64() {
            Some(seq) if seq > self.last_seq => seq,
            _ => return,
        };
        self.last_seq = seq;
        let card = event["cardId"].as_str().and_then(parse_card);
        if let Some(c) = &card {
            if let Some(id) = c["id"].as_str() {
                self.seen.insert(id.to_string());
            }
        }
        if let Some(id) = event["targetCardId"].as_str() {
            self.seen.insert(id.to_string());
        }
        match event["action"].as_str().unwrap_or("") {
            "attack" | "transfer" => {
                if let Some(c) = card {
                    let id = c["id"].as_str().unwrap_or("").to_string();
                    if !self.pending.iter().any(|(a, _)| a["id"] == id.as_str()) {
                        self.pending.push((c, None));
                    }
                }
            }
            "defend" => {
                let target = event["targetCardId"].as_str().unwrap_or("");
                let slot = self
                    .pending
                    .iter()
                    .position(|(a, d)| a["id"] == target && d.is_none())
                    .or_else(|| self.pending.iter().position(|(_, d)| d.is_none()));
                if let (Some(index), Some(c)) = (slot, card) {
                    self.pending[index].1 = Some(c);
                }
            }
            "pass" => {
                if !self.pending.is_empty() && self.pending.iter().all(|(_, d)| d.is_some()) {
                    for (attack, defense) in self.pending.drain(..) {
                        if let Some(id) = attack["id"].as_str() {
                            self.dead.insert(id.to_string());
                        }
                        if let Some(id) = defense.and_then(|d| d["id"].as_str().map(str::to_string))
                        {
                            self.dead.insert(id);
                        }
                    }
                }
            }
            "take" => self.pending.clear(),
            "start" | "cancel" => {
                self.dead.clear();
                self.pending.clear();
            }
            _ => {}
        }
    }

    fn unseen(&self, st: &Value) -> Vec<String> {
        let size = st["room"]["deckSize"].as_u64().unwrap_or(36);
        let ranks: Vec<&str> = if size == 52 {
            RANKS.to_vec()
        } else {
            RANKS[4..].to_vec()
        };
        let mut visible: HashSet<String> = self.seen.clone();
        for row in table(st) {
            if let Some(id) = row["attack"]["id"].as_str() {
                visible.insert(id.to_string());
            }
            if let Some(id) = row["defense"]["id"].as_str() {
                visible.insert(id.to_string());
            }
        }
        for c in hand(st) {
            if let Some(id) = c["id"].as_str() {
                visible.insert(id.to_string());
            }
        }
        if let Some(id) = st["trumpCard"]["id"].as_str() {
            visible.insert(id.to_string());
        }
        let mut unseen = Vec::new();
        for suit in SUITS {
            for rank in &ranks {
                let id = format!("{}{}", rank, suit);
                if !visible.contains(&id) {
                    unseen.push(id);
                }
            }
        }
        unseen
    }

    pub fn unseen_trumps(&self, st: &Value) -> usize {
        let trump = st["trump"].as_str().unwrap_or("");
        if trump.is_empty() {
            return 0;
        }
        self.unseen(st)
            .iter()
            .filter(|id| id.ends_with(trump))
            .count()
    }

    pub fn summary(&self, st: &Value) -> Value {
        if !st.is_object() || st["room"].is_null() {
            return json!({"unseen":0,"trumps":0,"out":0,"line":"журнал пуст"});
        }
        let unseen = self.unseen(st);
        let trumps = self.unseen_trumps(st);
        let deck = st["deck"].as_u64().unwrap_or(0) as usize;
        let in_hands = unseen.len().saturating_sub(deck);
        let line = format!(
            "Скрыто {}: в колоде {}, в чужих руках {} · козырей не видел {} · вне игры {}",
            unseen.len(),
            deck,
            in_hands,
            trumps,
            self.dead.len()
        );
        json!({"unseen":unseen.len(),"trumps":trumps,"out":self.dead.len(),"line":line})
    }
}

fn defend_decision(st: &Value, ledger: Option<&Ledger>) -> Option<Value> {
    let options = defense_options(st);
    let chosen = match options.iter().find(|(_, c)| !is_trump(c, st)) {
        Some(cheap) => cheap,
        None => {
            let trumps_in_hand = hand(st).iter().filter(|c| is_trump(c, st)).count();
            let spend = trumps_in_hand >= 2
                || st["deck"].as_u64().unwrap_or(0) == 0
                || unbeaten(st) >= 3
                || ledger.map(|l| l.unseen_trumps(st) <= 1).unwrap_or(false);
            if !spend {
                return None;
            }
            options.first()?
        }
    };
    let (row, card) = chosen;
    let reason = if is_trump(card, st) {
        "козырь допустим"
    } else {
        "младшая некозырная"
    };
    Some(
        json!({"action":"defend","cardId":card["id"],"targetCardId":row["attack"]["id"],"reason":reason}),
    )
}

pub fn choose_move(st: &Value, ledger: Option<&Ledger>) -> Option<Value> {
    if st["room"]["status"] != "playing"
        || st["you"]["finished"] == true
        || st["you"]["left"] == true
    {
        return None;
    }
    let defending = st["defenderId"] == st["you"]["id"];
    if st["takePending"] == true {
        if defending {
            return None;
        }
        let dumps = dump_options(st);
        if st["canAttack"] == true {
            if let Some(card) = dumps.first() {
                return Some(
                    json!({"action":"attack","cardId":card["id"],"reason":"догрузка под «беру»"}),
                );
            }
        }
        return if st["canPass"] == true {
            Some(json!({"action":"pass","reason":"закрыть кон"}))
        } else {
            None
        };
    }
    if unbeaten(st) > 0 {
        if !defending {
            return None;
        }
        if st["canTransfer"] == true {
            let transfers = transfer_options(st);
            let cheap_defense = defense_options(st).iter().any(|(_, c)| !is_trump(c, st));
            if let Some(card) = transfers.first() {
                if !cheap_defense || unbeaten(st) >= 2 {
                    return Some(
                        json!({"action":"transfer","cardId":card["id"],"reason":"перевод выгоднее защиты"}),
                    );
                }
            }
        }
        if let Some(defense) = defend_decision(st, ledger) {
            return Some(defense);
        }
        return if st["canTake"] == true {
            Some(json!({"action":"take","reason":"отбиться невыгодно"}))
        } else {
            None
        };
    }
    let choices = attack_choices(st);
    if let Some(card) = choices.first() {
        return Some(json!({"action":"attack","cardId":card["id"],"reason":"атака"}));
    }
    if st["canPass"] == true {
        return Some(json!({"action":"pass","reason":"закончить кон"}));
    }
    None
}

#[cfg(test)]
mod tests;
