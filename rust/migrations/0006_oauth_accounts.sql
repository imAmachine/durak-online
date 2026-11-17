CREATE TABLE legacy_accounts (
  client_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  balance INTEGER NOT NULL DEFAULT 1000 CHECK(balance >= 0),
  games INTEGER NOT NULL DEFAULT 0 CHECK(games >= 0),
  wins INTEGER NOT NULL DEFAULT 0 CHECK(wins >= 0),
  losses INTEGER NOT NULL DEFAULT 0 CHECK(losses >= 0)
);
CREATE TABLE oauth_identities (
  provider TEXT NOT NULL,
  subject TEXT NOT NULL,
  client_id TEXT NOT NULL UNIQUE,
  PRIMARY KEY(provider, subject)
);
