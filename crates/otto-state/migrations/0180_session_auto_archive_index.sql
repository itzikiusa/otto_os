-- The cross-workspace sweep reads only IDs and eligibility timestamps. Existing
-- workspace-leading indexes cannot cover it; keyset order survives row updates.
CREATE INDEX idx_sessions_auto_archive ON sessions(id, last_active_at)
    WHERE archived = 0 AND kind = 'agent';
