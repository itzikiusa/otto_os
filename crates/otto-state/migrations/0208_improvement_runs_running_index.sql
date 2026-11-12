-- Admission check for improvement runs (`ImprovementsRepo::has_running` /
-- `create_run`): "does this workspace have a run with status = 'running'?".
-- It runs ~30 s after every idle episode of a watched session, and the only
-- index was (workspace_id, started_at DESC), so each check walked every run
-- the workspace ever had (thousands on a long-running install). A partial
-- index holds just the running rows — at most one per workspace — so the
-- check is a single probe. Additive only: an older build ignores it.
CREATE INDEX IF NOT EXISTS idx_improvement_runs_running
    ON improvement_runs(workspace_id) WHERE status = 'running';
