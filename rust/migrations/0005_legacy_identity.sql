-- Bind legacy client IDs to a one-way token digest across server restarts.
CREATE TABLE legacy_identity (
  client_id TEXT PRIMARY KEY,
  token_hash TEXT NOT NULL
);
