-- When a workflow trigger's schedule was last (re)armed: created, re-enabled
-- after a pause, or given a new cadence/timezone/expression. Same fix as
-- 0147 for scheduled tasks: the trigger scheduler's due check never looks
-- before it, so re-enabling a trigger after a week no longer fires the missed
-- run (with its worktrees and Slack delivery) within a minute, and a new
-- "daily at 09:00" trigger enabled at 15:00 waits for tomorrow 09:00.
-- NULL (every pre-existing row) keeps the old behaviour: last_run alone.
ALTER TABLE workflow_triggers ADD COLUMN armed_at TEXT;
