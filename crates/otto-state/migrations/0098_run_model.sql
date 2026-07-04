-- Run with Otto: per-run model override ("" = provider default). Handed to the
-- executing agent (`--model` for single-agent / goal-loop executors).
ALTER TABLE otto_runs ADD COLUMN model TEXT NOT NULL DEFAULT '';
