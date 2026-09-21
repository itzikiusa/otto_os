-- Durable source cursors and bounded-duration ownership, never transcript bodies.
CREATE TABLE learning_checkpoints (
    source TEXT PRIMARY KEY,
    checkpoint_json TEXT NOT NULL DEFAULT 'null',
    lease_token TEXT,
    lease_until TEXT,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_learning_trail ON agent_trail(session_id, kind, id);
