-- 0061: custom-plugin runtime.
--
-- Two core tables that make plugins first-class without touching the closed
-- `Feature` enum or the global sqlx migration sequence:
--
--   plugin_feature_grants  — the string-keyed (by plugin slug) RBAC axis,
--                            parallel to user_feature_grants (0041). None = no row.
--   plugin_migrations      — per-plugin migration ledger for the runtime migrator
--                            (otto_plugin_sdk::run_migrations); disjoint from sqlx's
--                            own _sqlx_migrations table.

CREATE TABLE plugin_feature_grants (
  user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
  plugin_key TEXT NOT NULL,
  capability TEXT NOT NULL CHECK (capability IN ('view','edit','admin')),
  PRIMARY KEY (user_id, plugin_key)
);

CREATE TABLE plugin_migrations (
  plugin_key TEXT    NOT NULL,
  version    INTEGER NOT NULL,
  name       TEXT    NOT NULL,
  applied_at TEXT    NOT NULL,
  PRIMARY KEY (plugin_key, version)
);
