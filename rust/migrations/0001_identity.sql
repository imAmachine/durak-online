PRAGMA foreign_keys = ON;

CREATE TABLE users (
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  disabled_at INTEGER,
  CHECK (length(id) > 0),
  CHECK (disabled_at IS NULL OR disabled_at >= created_at)
);

CREATE TABLE sessions (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  token_hash TEXT NOT NULL UNIQUE,
  expires_at INTEGER NOT NULL,
  revoked_at INTEGER,
  last_seen_at INTEGER NOT NULL,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
  CHECK (length(token_hash) > 0),
  CHECK (expires_at > 0),
  CHECK (last_seen_at > 0),
  CHECK (revoked_at IS NULL OR revoked_at > 0)
);

CREATE INDEX sessions_user_id_idx ON sessions(user_id);
CREATE INDEX sessions_expiry_idx ON sessions(expires_at) WHERE revoked_at IS NULL;

CREATE TABLE profiles (
  user_id TEXT PRIMARY KEY,
  display_name TEXT NOT NULL,
  avatar_tint TEXT,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
  CHECK (length(trim(display_name)) > 0),
  CHECK (length(display_name) <= 80),
  CHECK (avatar_tint IS NULL OR length(avatar_tint) <= 32)
);
