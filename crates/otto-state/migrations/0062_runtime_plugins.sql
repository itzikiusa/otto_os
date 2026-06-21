-- 0062: runtime (out-of-process) plugins registry.
--
-- Each row is an installed external plugin Otto supervises as a child process and
-- reverse-proxies at /api/v1/plugins/<slug>/*. `token` is the per-plugin secret the
-- sidecar presents to the scoped host API. `enabled` gates whether it is spawned.
-- (RBAC for user access stays in plugin_feature_grants from 0061.)

CREATE TABLE plugins (
  slug         TEXT PRIMARY KEY,
  name         TEXT NOT NULL,
  icon         TEXT NOT NULL,
  version      TEXT NOT NULL,
  description  TEXT NOT NULL,
  source       TEXT NOT NULL,           -- absolute plugin directory (under the plugins home)
  exec_json    TEXT NOT NULL,           -- JSON array argv, run from `source`
  ui_dir       TEXT,                    -- iframe assets dir relative to `source` (NULL = no UI)
  health       TEXT NOT NULL,           -- health path on the sidecar (e.g. /health)
  enabled      INTEGER NOT NULL DEFAULT 0,
  token        TEXT NOT NULL,           -- per-plugin host-API bearer secret
  installed_at TEXT NOT NULL
);
