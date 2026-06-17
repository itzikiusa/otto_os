-- Sections are now scoped so the DB Explorer keeps its own section tree,
-- independent from the Connections page. Existing sections belong to the
-- Connections page ('connections'); the DB Explorer uses 'db'. Position is
-- still per sibling group, now additionally partitioned by scope.
ALTER TABLE connection_sections ADD COLUMN scope TEXT NOT NULL DEFAULT 'connections';
CREATE INDEX idx_connection_sections_ws_scope ON connection_sections(workspace_id, scope);
