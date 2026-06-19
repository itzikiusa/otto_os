-- Memory sharing: each memory is either `shared` (visible to all workspace
-- members — the team default) or `private` (visible only to its creator).
ALTER TABLE memories ADD COLUMN visibility TEXT NOT NULL DEFAULT 'shared';
CREATE INDEX idx_memories_vis ON memories(workspace_id, visibility, created_by);
