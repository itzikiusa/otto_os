-- When a schedule was last (re)armed: created, re-enabled after a pause, or
-- given a new cadence/timezone. The scheduler's due check never looks before
-- it, so turning a task back on after a week no longer fires (and delivers)
-- the missed occurrence within a minute, and a new "daily at 09:00" created
-- at 15:00 waits for tomorrow 09:00 — the time its row already shows.
-- NULL (every pre-existing row) keeps the old behaviour: last_run_at alone.
ALTER TABLE scheduled_tasks ADD COLUMN armed_at TEXT;
ALTER TABLE personal_agent_schedules ADD COLUMN armed_at TEXT;
