-- Structured agent runtime: linked threads. A thread an agent opened from
-- another thread (`POST /runtime-sessions/{id}/peers`) records the group's
-- ROOT thread here, so Threads shows the group together and the members may
-- message each other. Nullable, no DEFAULT needed: a rolled-back build
-- ignores it.
ALTER TABLE agent_runtime_sessions ADD COLUMN parent_id TEXT;
