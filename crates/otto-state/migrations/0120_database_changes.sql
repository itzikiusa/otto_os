-- Reviewed changes are immutable after submission. History retains every revision.
CREATE TABLE database_changes (
 id TEXT PRIMARY KEY, author_id TEXT NOT NULL, real_author_id TEXT NOT NULL,
 title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '', script TEXT NOT NULL,
 targets_json TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 1,
 status TEXT NOT NULL CHECK(status IN ('draft','validated','awaiting_review','approved','running','succeeded','failed','partially_applied','outcome_unknown','rejected','cancelled')),
 content_hash TEXT NOT NULL DEFAULT '', executor_id TEXT,
 validation_json TEXT NOT NULL DEFAULT '{}', approved_by TEXT,
 approved_real_by TEXT, approval_hash TEXT, cancellation_requested INTEGER NOT NULL DEFAULT 0,
 created_at TEXT NOT NULL, updated_at TEXT NOT NULL
);
CREATE INDEX database_changes_author ON database_changes(author_id,created_at);
CREATE TABLE database_change_events (
 id TEXT PRIMARY KEY, change_id TEXT NOT NULL REFERENCES database_changes(id),
 revision INTEGER NOT NULL, action TEXT NOT NULL, actor_id TEXT NOT NULL,
 real_actor_id TEXT NOT NULL, data_json TEXT NOT NULL, created_at TEXT NOT NULL
);
CREATE INDEX database_change_events_change ON database_change_events(change_id,created_at);
CREATE TABLE database_change_attempts (
 id TEXT PRIMARY KEY, change_id TEXT NOT NULL REFERENCES database_changes(id),
 connection_id TEXT NOT NULL, node TEXT, script TEXT NOT NULL,
 state TEXT NOT NULL CHECK(state IN ('queued','running','succeeded','failed','partially_applied','cancelled','outcome_unknown')),
 executor_id TEXT NOT NULL, content_hash TEXT NOT NULL, policy_revision INTEGER NOT NULL,
 connection_fingerprint TEXT NOT NULL, ordinal INTEGER NOT NULL,
 summary TEXT, started_at TEXT, finished_at TEXT,
 UNIQUE(change_id,ordinal)
);
-- Unknown attempts deliberately retain a lock until operator reconciliation.
CREATE UNIQUE INDEX database_change_target_lock ON database_change_attempts(connection_id,COALESCE(node,''))
 WHERE state IN ('queued','running','outcome_unknown');
