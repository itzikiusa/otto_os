-- Structured agent runtime: the session's access mode (T3 Code's RuntimeMode,
-- `approval-required` | `auto-accept-edits` | `full-access`), sticky like
-- `model`. A new column with a DEFAULT: a rolled-back build ignores it.
ALTER TABLE agent_runtime_sessions ADD COLUMN access_mode TEXT NOT NULL DEFAULT 'approval-required';
