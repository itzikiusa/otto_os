-- 0096_workflow_instructions.sql
-- Standing instructions every workflow step follows (design 2026-07-03).
-- Versions carry it so snapshot/restore round-trips instructions with the graph.
ALTER TABLE workflows         ADD COLUMN instructions TEXT NOT NULL DEFAULT '';
ALTER TABLE workflow_versions ADD COLUMN instructions TEXT NOT NULL DEFAULT '';
