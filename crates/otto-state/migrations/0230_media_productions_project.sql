-- Create projects (contract v1): every creation belongs to one project, the
-- id of the project's first creation. NULL (every older row, and a row the
-- previous build writes again: its INSERT OR REPLACE writes NULL) is its own
-- project, so the previous build reads and writes the table as before.
ALTER TABLE media_productions ADD COLUMN project_id TEXT;
CREATE INDEX IF NOT EXISTS idx_media_productions_project ON media_productions (project_id);
