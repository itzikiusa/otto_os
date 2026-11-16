-- Workbench → Scratches: scratch files are now real files in the owner's
-- Scratches folder (opened as a project in the IDE). Each live doc is copied
-- there ONCE; `exported_path` records where it went (relative to that
-- folder), so the copy is idempotent and a doc the old build creates after a
-- rollback is still picked up by the next pass. Additive only: the old build
-- ignores the column, and every workbench table stays as the archive.
ALTER TABLE workbench_docs ADD COLUMN exported_path TEXT;
