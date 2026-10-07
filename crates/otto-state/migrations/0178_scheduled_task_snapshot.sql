-- Immutable admitted definition, including internal generation counters, used
-- after workflow hand-off recovery. NULL keeps older writers compatible and
-- identifies legacy runs whose delivery/cursor ownership cannot be proven.
ALTER TABLE scheduled_task_runs ADD COLUMN admitted_task_json TEXT;
