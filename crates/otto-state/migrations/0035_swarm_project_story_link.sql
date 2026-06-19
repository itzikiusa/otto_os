-- Product↔Swarm bridge: a swarm project may be created from a Product story
-- (Plan → Swarm). Back-link the source story so the project knows its origin
-- and the story can show its linked project. Nullable: most projects have no
-- story. Indexed for the reverse lookup (story_id → project).
ALTER TABLE swarm_projects ADD COLUMN story_id TEXT;
CREATE INDEX idx_swarm_projects_story ON swarm_projects(story_id);
