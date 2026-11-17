PRAGMA foreign_keys = ON;

CREATE TABLE wallets (
  user_id TEXT PRIMARY KEY,
  balance INTEGER NOT NULL DEFAULT 0,
  version INTEGER NOT NULL DEFAULT 0,
  updated_at INTEGER NOT NULL,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
  CHECK (balance >= 0),
  CHECK (version >= 0),
  CHECK (updated_at > 0)
);

CREATE TABLE wallet_ledger (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL,
  game_id TEXT,
  kind TEXT NOT NULL,
  amount INTEGER NOT NULL,
  operation_id TEXT NOT NULL UNIQUE,
  created_at INTEGER NOT NULL,
  FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE RESTRICT,
  FOREIGN KEY (game_id) REFERENCES matches(id) ON DELETE RESTRICT,
  CHECK (kind IN ('stake', 'payout', 'refund', 'admin_adjustment')),
  CHECK (amount != 0),
  CHECK (created_at > 0),
  CHECK (game_id IS NOT NULL OR kind = 'admin_adjustment')
);

CREATE INDEX wallet_ledger_user_id_created_at_idx ON wallet_ledger(user_id, created_at);
CREATE INDEX wallet_ledger_game_id_idx ON wallet_ledger(game_id);

CREATE TABLE admin_audit (
  id TEXT PRIMARY KEY,
  actor_user_id TEXT,
  operation_kind TEXT NOT NULL,
  target_user_id TEXT,
  payload_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  FOREIGN KEY (actor_user_id) REFERENCES users(id) ON DELETE SET NULL,
  FOREIGN KEY (target_user_id) REFERENCES users(id) ON DELETE SET NULL,
  CHECK (length(operation_kind) > 0),
  CHECK (length(payload_json) > 0),
  CHECK (created_at > 0)
);
