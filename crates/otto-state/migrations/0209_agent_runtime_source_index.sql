-- Embedded structured-runtime threads (a PR review's Ask / Explain / Fix, a
-- connection's diagnosis) carry `meta.source` (+ `meta.origin`). Two reads
-- select on it: the page finding its thread again
-- (`GET /workspaces/{id}/runtime-sessions?source=&origin_id=`) and the hourly
-- auto-close sweep (`embedded_thread_auto_close_hours`, every workspace).
-- Without an index both parse the meta JSON of every runtime row. An index on
-- the expression lets them probe instead; the queries name it exactly as
-- written here. `meta` is only ever written by the daemon as serialized JSON
-- (or NULL), so the expression cannot fail on a row. Additive only: an older
-- build ignores it.
CREATE INDEX IF NOT EXISTS idx_agent_runtime_sessions_source
    ON agent_runtime_sessions(json_extract(meta, '$.source'), workspace_id);
