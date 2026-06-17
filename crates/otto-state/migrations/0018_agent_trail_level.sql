-- Severity of a trail entry, so the UI can color failures/warnings and the
-- daemon can raise notifications off the trail. Existing rows default to 'info'.
ALTER TABLE agent_trail
    ADD COLUMN level TEXT NOT NULL DEFAULT 'info'
    CHECK (level IN ('info', 'warn', 'error'));
