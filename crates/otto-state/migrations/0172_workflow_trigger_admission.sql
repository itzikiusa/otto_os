-- An admission epoch, distinct from a completed occurrence's cursor. Even
-- retiming away/back or disabling/re-enabling invalidates a captured tick.
ALTER TABLE workflow_triggers ADD COLUMN admission_generation INTEGER NOT NULL DEFAULT 0;
