-- Enforce at most one swarm project per Product story. The Plan→Swarm hand-off
-- (`story_to_swarm`) pre-checks for an existing linked project and returns it
-- idempotently, but two concurrent hand-offs can both pass that check and create
-- duplicate linked projects. A partial unique index closes the race: only rows
-- with a non-NULL `story_id` are constrained, so the many projects with no story
-- (NULL) are unaffected. On conflict, the hand-off falls back to the existing row.
CREATE UNIQUE INDEX idx_swarm_projects_story_unique
    ON swarm_projects(story_id)
    WHERE story_id IS NOT NULL;
