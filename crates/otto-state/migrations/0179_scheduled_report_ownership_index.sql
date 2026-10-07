-- Retention checks whether a report is still owned by another history row.
-- Legacy same-second reports can share a path; avoid scanning all run history
-- once per deleted report while preserving those retained references.
CREATE INDEX idx_str_report_path ON scheduled_task_runs(report_path)
    WHERE report_path IS NOT NULL;
