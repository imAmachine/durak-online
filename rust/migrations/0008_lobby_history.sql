CREATE TABLE lobby_history (
  ordinal INTEGER PRIMARY KEY AUTOINCREMENT,
  game_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  room TEXT NOT NULL,
  mode TEXT NOT NULL CHECK(mode IN ('classic', 'transfer')),
  at INTEGER NOT NULL CHECK(at >= 0),
  loser TEXT NOT NULL,
  bet INTEGER NOT NULL CHECK(bet >= 0),
  UNIQUE(game_id, seq),
  FOREIGN KEY(game_id, seq) REFERENCES durable_game_transitions(game_id, seq) ON DELETE RESTRICT
);
