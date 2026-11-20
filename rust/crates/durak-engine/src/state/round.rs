use super::{DurakState, Finished, Player, Status, HAND_LIMIT};

impl DurakState {
    pub(super) fn refill(&mut self) {
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
    pub(super) fn settle(&mut self) {
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
}
