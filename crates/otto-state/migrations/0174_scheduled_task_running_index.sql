-- Admission only needs active runs. Avoid scanning all completed history for
-- every scheduled dispatch once a task has accumulated many executions.
CREATE INDEX idx_str_running_task ON scheduled_task_runs(task_id) WHERE status = 'running';
