-- Feature: Scheduled Tasks v2 — multi-provider runs (codex/agy/shell/custom), an
-- IANA timezone for daily/weekly/cron schedules, an optional cron cadence, a
-- visible session row per run, an optional per-task sandbox worktree, a
-- schedule→workflow handoff, a retry policy, "only notify on meaningful change",
-- and an optional proof pack attached to each run report.
--
-- Append-only ALTER ADD COLUMN over 0084. Every column is defaulted so existing
-- tasks behave exactly as before (provider stays whatever it was, timezone=UTC,
-- sandbox=none, no retries, no notify-on-change gate, no proof). The `cron`
-- cadence and `workflow_id` live in the existing schedule_json / a new column.

ALTER TABLE scheduled_tasks ADD COLUMN timezone TEXT NOT NULL DEFAULT 'UTC';          -- IANA tz for daily/weekly/cron
ALTER TABLE scheduled_tasks ADD COLUMN workflow_id TEXT;                              -- kind='workflow' target
ALTER TABLE scheduled_tasks ADD COLUMN sandbox TEXT NOT NULL DEFAULT 'none';          -- none | worktree
ALTER TABLE scheduled_tasks ADD COLUMN max_retries INTEGER NOT NULL DEFAULT 0;        -- 0..=5 agent retries
ALTER TABLE scheduled_tasks ADD COLUMN notify_on_change INTEGER NOT NULL DEFAULT 0;   -- deliver only if report changed
ALTER TABLE scheduled_tasks ADD COLUMN attach_proof INTEGER NOT NULL DEFAULT 0;       -- build a proof pack per run

ALTER TABLE scheduled_task_runs ADD COLUMN report_hash TEXT;                          -- normalized content hash (notify-on-change)
ALTER TABLE scheduled_task_runs ADD COLUMN proof_pack_id TEXT;                        -- attached proof pack id
ALTER TABLE scheduled_task_runs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 1;       -- agent run attempts (retry policy)
ALTER TABLE scheduled_task_runs ADD COLUMN skipped_delivery INTEGER NOT NULL DEFAULT 0; -- delivery suppressed (no change)
ALTER TABLE scheduled_task_runs ADD COLUMN workflow_run_id TEXT;                      -- kind='workflow' run link
