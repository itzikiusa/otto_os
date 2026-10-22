-- Workflow run identity (PC-3, access-control-3-02): who last set a trigger's
-- spec. A trigger, schedule, webhook or chat run acts as the LATEST changer of
-- what it runs — this column or the newest `workflow_versions.created_by` —
-- instead of always the workflow's creator. Nullable, additive: NULL means "the
-- creator", which is exactly how the previous build reads every trigger.
-- `updated_by_at` is when that spec change happened (the `fmt` timestamp
-- `workflow_versions.created_at` uses), so "later" is comparable.
ALTER TABLE workflow_triggers ADD COLUMN updated_by TEXT;
ALTER TABLE workflow_triggers ADD COLUMN updated_by_at TEXT;
