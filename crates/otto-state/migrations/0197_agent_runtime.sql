-- Structured agent runtime (docs/features/threads-structured-runtime.md):
-- agent CLIs driven over a stdio protocol instead of a PTY. Kept in their OWN
-- tables, not `sessions`, so none of the PTY session machinery (attach,
-- ensure_live, idle suspend, restart, prune) can ever act on them.
--   * agent_runtime_sessions — one row per runtime session. `seq_reserved` is
--     the sequence high-water mark a restarted daemon resumes ABOVE, so a
--     sequence a client saw in an unpersisted event is never reused.
--   * agent_runtime_entities — the projection: the latest state of every
--     entity (provider session/thread, turns, turn items, runtime requests,
--     plans) as JSON, keyed by kind + id, with the sequence of its last
--     update (the T3 Code projection model). Streaming text is written once it
--     settles; boot recovery cancels what a previous life left open.
-- New tables only: a rolled-back build never names them.
CREATE TABLE agent_runtime_sessions (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    provider            TEXT NOT NULL,
    title               TEXT NOT NULL,
    cwd                 TEXT NOT NULL,
    model               TEXT,
    native_thread_id    TEXT,
    closed              INTEGER NOT NULL DEFAULT 0,
    created_by          TEXT NOT NULL REFERENCES users(id),
    created_at          TEXT NOT NULL,
    last_active_at      TEXT NOT NULL,
    last_seq            INTEGER NOT NULL DEFAULT 0,
    seq_reserved        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_agent_runtime_sessions_ws ON agent_runtime_sessions(workspace_id, created_by);

CREATE TABLE agent_runtime_entities (
    session_id   TEXT NOT NULL REFERENCES agent_runtime_sessions(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,
    entity_id    TEXT NOT NULL,
    sequence     INTEGER NOT NULL,
    payload_json TEXT NOT NULL,
    PRIMARY KEY (session_id, kind, entity_id)
);
