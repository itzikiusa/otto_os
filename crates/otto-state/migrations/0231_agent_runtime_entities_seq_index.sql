-- The latest turn of a runtime session by sequence: every session list and
-- get reads it per row (`session_cols!` in agent_runtime.rs). Without this
-- index each row sorted all of its session's turns in a temp B-tree
-- (264 sessions x 60 turns: 28 ms per list, 1 ms with it). Index only: a
-- rolled-back build ignores it.
CREATE INDEX IF NOT EXISTS idx_agent_runtime_entities_seq
    ON agent_runtime_entities(session_id, kind, sequence);
