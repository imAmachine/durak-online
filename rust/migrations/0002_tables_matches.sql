PRAGMA foreign_keys = ON;

CREATE TABLE tables (
  id TEXT PRIMARY KEY,
  owner_user_id TEXT NOT NULL,
  room_code TEXT NOT NULL UNIQUE,
  mode TEXT NOT NULL,
  status TEXT NOT NULL,
  lobby_seq INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  closed_at INTEGER,
  FOREIGN KEY (owner_user_id) REFERENCES users(id) ON DELETE RESTRICT,
  CHECK (length(id) > 0),
  CHECK (length(room_code) BETWEEN 4 AND 32),
  CHECK (mode IN ('classic', 'transfer')),
  CHECK (status IN ('waiting', 'playing', 'finished', 'closed')),
  CHECK (lobby_seq >= 0),
  CHECK (updated_at >= created_at),
  CHECK (closed_at IS NULL OR closed_at >= created_at)
);

CREATE INDEX tables_owner_user_id_idx ON tables(owner_user_id);
CREATE INDEX tables_status_idx ON tables(status);

CREATE TABLE matches (
  id TEXT PRIMARY KEY,
  table_id TEXT NOT NULL,
  game_type TEXT NOT NULL,
  rules_version TEXT NOT NULL,
  mode TEXT NOT NULL,
  status TEXT NOT NULL,
  state_json TEXT NOT NULL,
  seq INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  finished_at INTEGER,
  FOREIGN KEY (table_id) REFERENCES tables(id) ON DELETE RESTRICT,
  CHECK (length(game_type) > 0),
  CHECK (length(rules_version) > 0),
  CHECK (mode IN ('classic', 'transfer')),
  CHECK (status IN ('created', 'active', 'finished', 'aborted')),
  CHECK (length(state_json) > 0),
  CHECK (seq >= 0),
  CHECK (updated_at >= created_at),
  CHECK (finished_at IS NULL OR finished_at >= created_at)
);

CREATE INDEX matches_table_id_idx ON matches(table_id);
CREATE INDEX matches_status_idx ON matches(status);

CREATE TABLE game_players (
  match_id TEXT NOT NULL,
  user_id TEXT NOT NULL,
  seat INTEGER NOT NULL,
  status TEXT NOT NULL,
  result TEXT,
  payout INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY (match_id, user_id),
  UNIQUE (match_id, seat),
  FOREIGN KEY (match_id) REFERENCES matches(id) ON DELETE CASCADE,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE RESTRICT,
  CHECK (seat >= 0),
  CHECK (status IN ('joined', 'active', 'left', 'finished')),
  CHECK (result IS NULL OR result IN ('winner', 'loser', 'draw')),
  CHECK (payout >= 0)
);

CREATE INDEX game_players_user_id_idx ON game_players(user_id);
