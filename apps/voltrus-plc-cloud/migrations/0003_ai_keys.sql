CREATE TABLE ai_keys (
  user_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  key_encrypted TEXT NOT NULL,
  key_iv TEXT NOT NULL,
  masked TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (user_id, provider)
);

CREATE TABLE ai_usage (
  user_id TEXT NOT NULL,
  window_start TEXT NOT NULL,
  count INTEGER NOT NULL,
  PRIMARY KEY (user_id, window_start)
);
