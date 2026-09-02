-- Workflow resume-on-restart (design docs/sessions-overview-review-2026-09-01.md).
--
-- A daemon restart used to hard-fail every in-flight run. Per-node progress is
-- already persisted in `nodes_json` after every step, so the startup
-- reconciler can now re-enter an interrupted run instead:
--   * `resume_scope_json` persists the re-entry scope (`start_node`,
--     `only_node`, `adopt_start`) — previously a retry-a-step's scope lived
--     only in the dead process's memory, which forced failing every
--     pending-with-progress row. Cleared when the run reaches a terminal
--     status.
--   * `interrupted_at` / `resume_attempts` record the restart-resume history;
--     the reconciler caps automatic resumes (crash-loop guard).
--   * `workflows.on_restart` is the per-workflow policy: 'resume' (default)
--     re-enters interrupted runs, 'fail' preserves the old hard-fail
--     behavior. Versions carry it so snapshot/restore round-trips the policy
--     with the graph (mirrors 0096's `instructions`).

ALTER TABLE workflow_runs ADD COLUMN resume_scope_json TEXT;
ALTER TABLE workflow_runs ADD COLUMN interrupted_at TEXT;
ALTER TABLE workflow_runs ADD COLUMN resume_attempts INTEGER NOT NULL DEFAULT 0;

ALTER TABLE workflows         ADD COLUMN on_restart TEXT NOT NULL DEFAULT 'resume'
    CHECK (on_restart IN ('resume', 'fail'));
ALTER TABLE workflow_versions ADD COLUMN on_restart TEXT NOT NULL DEFAULT 'resume'
    CHECK (on_restart IN ('resume', 'fail'));
