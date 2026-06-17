-- Per-session agent activity: a chronological "live trail" of what happened
-- (skills loaded, commands run, files touched, prompts, user notes) plus a
-- normalized task tracker (each provider's native task list — Claude TodoWrite,
-- etc. — mapped into one shape). Mirrors otto_core::domain::{TrailEvent,AgentTask}.
-- ULID string PKs, UTC RFC3339 timestamps, JSON in *_json columns. Rows cascade
-- away with their session.

CREATE TABLE agent_trail (
    id           TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    ts           TEXT NOT NULL,
    -- who produced the entry.
    source       TEXT NOT NULL CHECK (source IN ('user','agent','otto')),
    -- coarse category, drives the UI icon/grouping.
    kind         TEXT NOT NULL CHECK (kind IN
                   ('session','prompt','skill','command','tool','file','web','task','note','other')),
    summary      TEXT NOT NULL,
    -- optional structured payload (raw tool input, etc.), capped by the writer.
    detail_json  TEXT
);
CREATE INDEX idx_agent_trail_session ON agent_trail(session_id, ts DESC, id DESC);

CREATE TABLE agent_tasks (
    id           TEXT PRIMARY KEY,
    session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    -- provider-native id for the task when one exists (else NULL). Lets an
    -- incremental update target a row; the full-sync path ignores it.
    ext_id       TEXT,
    title        TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN
                   ('pending','in_progress','completed','blocked','cancelled')),
    position     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_agent_tasks_session ON agent_tasks(session_id, position);
