-- A run may settle only the scheduling occurrence it captured at dispatch.
-- Monotonic generations distinguish reschedule-away-and-back from no edit.
ALTER TABLE scheduled_tasks ADD COLUMN schedule_generation INTEGER NOT NULL DEFAULT 0;
