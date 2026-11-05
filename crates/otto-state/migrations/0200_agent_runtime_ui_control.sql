-- Structured agent runtime: the per-thread "agent UI control" grant (the
-- `otto.ui_*` tools that drive Otto beside the thread). The JSON grant record
-- `{enabled, by, at}` that terminal sessions keep in `sessions.meta.ui_control`.
-- Written only by `POST /sessions/{id}/ui-control`. Nullable: a rolled-back
-- build ignores it.
ALTER TABLE agent_runtime_sessions ADD COLUMN ui_control TEXT;
