-- User-defined sections to organize connection profiles within a workspace.
-- Sections nest into a tree via `parent_id` (NULL = top-level); deleting a
-- section cascades to its descendant sections.
CREATE TABLE connection_sections (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    parent_id    TEXT REFERENCES connection_sections(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_connection_sections_ws ON connection_sections(workspace_id);
CREATE INDEX idx_connection_sections_parent ON connection_sections(parent_id);

-- A connection optionally belongs to one section; deleting the section
-- drops its connections back to "ungrouped" (section_id = NULL).
ALTER TABLE connections
    ADD COLUMN section_id TEXT REFERENCES connection_sections(id) ON DELETE SET NULL;
