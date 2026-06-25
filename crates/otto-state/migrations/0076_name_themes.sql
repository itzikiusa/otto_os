-- Session name THEMES: auto-name new agent sessions from a fame-ordered pool
-- (e.g. "Ronaldo", "Messi") instead of "claude #3".
--
-- Built-in themes (footballers, scientists, …) are compiled into the daemon and
-- are NOT stored here. This migration persists only the two user-owned bits:
--   * `name_themes`        — a user's CUSTOM ordered name lists (e.g. family names).
--   * `name_theme_active`  — which theme each user has selected for new sessions.
CREATE TABLE name_themes (
    id          TEXT PRIMARY KEY,
    owner_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    label       TEXT NOT NULL,
    -- Ordered JSON array of name strings, e.g. ["Dad","Mom","Sister"].
    names_json  TEXT NOT NULL DEFAULT '[]',
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_name_themes_owner ON name_themes(owner_id, created_at);

-- Each user's active theme for auto-naming. `theme_id` is a built-in id
-- ("footballers"), a custom `name_themes.id`, or the sentinel "none" (the legacy
-- "{provider} #N" numbering). No row → the daemon's default theme.
CREATE TABLE name_theme_active (
    user_id     TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    theme_id    TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
