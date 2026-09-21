-- Preserve the existing project identity and every Swarm execution field.
-- An ordinary project has no swarm; Swarm queries keep filtering by swarm_id.
ALTER TABLE swarm_projects ADD COLUMN optional_swarm_id TEXT;
UPDATE swarm_projects SET optional_swarm_id = swarm_id;
DROP INDEX idx_swarm_projects_swarm;
ALTER TABLE swarm_projects DROP COLUMN swarm_id;
ALTER TABLE swarm_projects RENAME COLUMN optional_swarm_id TO swarm_id;
CREATE INDEX idx_swarm_projects_swarm ON swarm_projects(swarm_id);
CREATE INDEX idx_projects_workspace ON swarm_projects(workspace_id, updated_at);
ALTER TABLE swarm_projects ADD COLUMN instructions_md TEXT NOT NULL DEFAULT '';
ALTER TABLE swarm_projects ADD COLUMN references_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE swarm_projects ADD COLUMN memory_md TEXT NOT NULL DEFAULT '';
ALTER TABLE swarm_projects ADD COLUMN decisions_md TEXT NOT NULL DEFAULT '';
ALTER TABLE swarm_projects ADD COLUMN artifacts_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE swarm_projects ADD COLUMN context_version INTEGER NOT NULL DEFAULT 1;

-- Existing Swarm sessions join the same project. Explicit membership wins.
UPDATE sessions SET meta_json = json_set(meta_json, '$.project_id', (
    SELECT r.project_id FROM swarm_runs r
    JOIN swarm_projects p ON p.id = r.project_id AND p.workspace_id = sessions.workspace_id
    WHERE r.session_id = sessions.id ORDER BY r.enqueued_at DESC LIMIT 1
))
WHERE json_valid(meta_json) AND json_type(meta_json) = 'object'
  AND json_extract(meta_json, '$.project_id') IS NULL
  AND EXISTS (SELECT 1 FROM swarm_runs r JOIN swarm_projects p ON p.id = r.project_id
              WHERE r.session_id = sessions.id AND p.workspace_id = sessions.workspace_id);
CREATE INDEX idx_sessions_project ON sessions(workspace_id, json_extract(meta_json, '$.project_id'), last_active_at, id);
CREATE INDEX idx_sessions_project_owner ON sessions(workspace_id, json_extract(meta_json, '$.project_id'), created_by, last_active_at, id);
