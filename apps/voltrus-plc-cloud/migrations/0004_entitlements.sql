CREATE TABLE entitlements (
  user_id TEXT PRIMARY KEY REFERENCES users(id),
  plan TEXT NOT NULL DEFAULT 'free' CHECK (plan IN ('free', 'solo', 'team')),
  status TEXT NOT NULL DEFAULT 'active',
  period_end TEXT,
  polar_subscription_id TEXT,
  updated_at TEXT NOT NULL
);

CREATE TABLE team_seats (
  team_subscription_id TEXT NOT NULL,
  user_id TEXT NOT NULL PRIMARY KEY REFERENCES users(id),
  invited_by TEXT,
  created_at TEXT NOT NULL
);
