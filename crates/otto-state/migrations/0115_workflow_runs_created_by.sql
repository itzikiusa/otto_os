-- Feature: workflow runs act as the user who STARTED them.
--
-- A run's agent sessions used to be owned by the workflow's creator (the
-- engine resolved its acting user from `workflows.created_by`), so a
-- non-root editor who ran someone else's workflow could not attach to the
-- sessions the run spawned (owner-or-admin check) even with full Workflows +
-- Agents permissions. `created_by` records the starter; NULL = trigger /
-- schedule / chat-initiated run (falls back to the workflow's creator).
ALTER TABLE workflow_runs ADD COLUMN created_by TEXT;
