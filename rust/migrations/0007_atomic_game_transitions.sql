CREATE TABLE durable_game_states (
  game_id TEXT PRIMARY KEY,
  seq INTEGER NOT NULL CHECK(seq >= 0),
  state_json TEXT NOT NULL CHECK(length(state_json) > 0)
);
CREATE TABLE durable_game_transitions (
  game_id TEXT NOT NULL,
  seq INTEGER NOT NULL CHECK(seq >= 0),
  payload_json TEXT NOT NULL CHECK(length(payload_json) > 0),
  PRIMARY KEY(game_id, seq),
  FOREIGN KEY(game_id) REFERENCES durable_game_states(game_id) ON DELETE RESTRICT
);
