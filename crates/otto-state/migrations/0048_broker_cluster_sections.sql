-- User-defined sections to organize broker (Kafka) cluster profiles within a
-- workspace. Sections nest into a tree via `parent_id` (NULL = top-level);
-- deleting a section cascades to its descendant sections.
CREATE TABLE broker_cluster_sections (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    parent_id    TEXT REFERENCES broker_cluster_sections(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    position     INTEGER NOT NULL,
    created_by   TEXT NOT NULL REFERENCES users(id),
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_broker_cluster_sections_ws ON broker_cluster_sections(workspace_id);
CREATE INDEX idx_broker_cluster_sections_parent ON broker_cluster_sections(parent_id);

-- A cluster optionally belongs to one section; deleting the section drops its
-- clusters back to "ungrouped" (section_id = NULL). Global clusters
-- (workspace_id = NULL) are not assignable and always show ungrouped.
ALTER TABLE broker_clusters
    ADD COLUMN section_id TEXT REFERENCES broker_cluster_sections(id) ON DELETE SET NULL;
