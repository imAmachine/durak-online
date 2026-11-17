PRAGMA foreign_keys = ON;

CREATE TABLE game_events (
  id TEXT PRIMARY KEY,
  match_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  public_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  UNIQUE (match_id, seq),
  FOREIGN KEY (match_id) REFERENCES matches(id) ON DELETE CASCADE,
  CHECK (seq > 0),
  CHECK (length(public_json) > 0),
  CHECK (created_at > 0)
);

CREATE INDEX game_events_match_seq_idx ON game_events(match_id, seq);

CREATE TABLE outbox (
  id TEXT PRIMARY KEY,
  table_id TEXT NOT NULL,
  room_seq INTEGER NOT NULL,
  payload TEXT NOT NULL,
  delivered_at INTEGER,
  FOREIGN KEY (table_id) REFERENCES tables(id) ON DELETE CASCADE,
  UNIQUE (table_id, room_seq),
  CHECK (room_seq > 0),
  CHECK (length(payload) > 0),
  CHECK (delivered_at IS NULL OR delivered_at > 0)
);

CREATE INDEX outbox_pending_idx ON outbox(table_id, room_seq) WHERE delivered_at IS NULL;

CREATE TABLE game_history (
  game_id TEXT PRIMARY KEY,
  loser_user_id TEXT,
  summary_json TEXT NOT NULL,
  finished_at INTEGER NOT NULL,
  FOREIGN KEY (game_id) REFERENCES matches(id) ON DELETE CASCADE,
  FOREIGN KEY (loser_user_id) REFERENCES users(id) ON DELETE SET NULL,
  CHECK (length(summary_json) > 0),
  CHECK (finished_at > 0)
);
