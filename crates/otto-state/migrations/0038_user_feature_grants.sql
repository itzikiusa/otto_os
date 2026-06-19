-- Per-user per-feature capability grants (default-deny; no row = None).
-- Root users bypass this table entirely (GrantsRepo::capability_of returns Admin).
CREATE TABLE user_feature_grants (
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  feature    TEXT NOT NULL,                                   -- snake_case Feature
  capability TEXT NOT NULL CHECK (capability IN ('view','edit','admin')),
  PRIMARY KEY (user_id, feature)
);
