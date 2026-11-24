CREATE TABLE devices (
  id TEXT PRIMARY KEY,
  user_id TEXT NOT NULL REFERENCES users(id),
  name TEXT NOT NULL,
  pairing_token_hash TEXT NOT NULL,
  last_seen_at TEXT,
  created_at TEXT NOT NULL
);
