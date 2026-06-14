-- Persisted notifications surfaced in the notification center. Mirrors
-- otto_core::domain::Notice. ULID string PK, UTC RFC3339 timestamps, JSON in
-- *_json columns. De-duping on source_key is enforced in the repository.

CREATE TABLE notifications (
    id          TEXT PRIMARY KEY,
    created_at  TEXT NOT NULL,
    read        INTEGER NOT NULL DEFAULT 0,
    kind        TEXT NOT NULL CHECK (kind IN ('credential','session','system')),
    severity    TEXT NOT NULL CHECK (severity IN ('info','warn','error')),
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    source_key  TEXT,
    action_json TEXT
);
CREATE INDEX idx_notifications_created ON notifications(created_at DESC);
-- One live (non-dismissed) row per source_key: drives create()'s de-dupe.
CREATE UNIQUE INDEX idx_notifications_source_key ON notifications(source_key)
    WHERE source_key IS NOT NULL;

-- Singleton settings row (id = 1). Holds the JSON-encoded NotificationSettings.
CREATE TABLE notification_settings (
    id            INTEGER PRIMARY KEY CHECK (id = 1),
    settings_json TEXT NOT NULL
);
