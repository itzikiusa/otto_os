-- Workbench: a doc's visibility scope. `workspace` (the default — every doc
-- created before this migration) shows only in the workspace it was created
-- in; `global` shows in EVERY workspace of its owner (still owner-only).
-- Additive only: the old build ignores the column and keeps treating every
-- doc as workspace-scoped.
ALTER TABLE workbench_docs ADD COLUMN scope TEXT NOT NULL DEFAULT 'workspace';
CREATE INDEX IF NOT EXISTS idx_workbench_docs_owner_scope
    ON workbench_docs(owner_id, scope, deleted_at, updated_at);
