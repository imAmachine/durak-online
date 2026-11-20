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
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "♠" => Some(Self::Spades),
            "♥" => Some(Self::Hearts),
            "♦" => Some(Self::Diamonds),
            "♣" => Some(Self::Clubs),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Spades => "♠",
            Self::Hearts => "♥",
            Self::Diamonds => "♦",
            Self::Clubs => "♣",
        }
    }

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
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "2" => Some(Self::Two),
            "3" => Some(Self::Three),
            "4" => Some(Self::Four),
            "5" => Some(Self::Five),
            "6" => Some(Self::Six),
            "7" => Some(Self::Seven),
            "8" => Some(Self::Eight),
            "9" => Some(Self::Nine),
            "10" => Some(Self::Ten),
            "В" | "J" => Some(Self::Jack),
            "Д" | "Q" => Some(Self::Queen),
            "К" | "K" => Some(Self::King),
            "Т" | "A" => Some(Self::Ace),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Two => "2",
            Self::Three => "3",
            Self::Four => "4",
            Self::Five => "5",
            Self::Six => "6",
            Self::Seven => "7",
            Self::Eight => "8",
            Self::Nine => "9",
            Self::Ten => "10",
            Self::Jack => "В",
            Self::Queen => "Д",
            Self::King => "К",
            Self::Ace => "Т",
        }
    }

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
    pub(super) fn value(self) -> usize {
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

pub fn can_beat(attack: Card, defense: Card, trump: Suit) -> bool {
    if attack.suit == defense.suit {
        defense.rank.value() > attack.rank.value()
    } else {
        defense.suit == trump && attack.suit != trump
    }
}
