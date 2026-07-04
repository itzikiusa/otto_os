-- Skills Lab: multi-agent review of a SKILL.md package.
--
-- A `skill_reviews` row is one review of one skill. `static_json` holds the
-- deterministic native static-analysis report (always present once the run
-- starts). In "agents" mode, `agents_json` holds the live per-agent state
-- (mirrors pr_reviews.agents_json — one row per provider agent plus a trailing
-- summarizer row, updated per array index during the run so the UI's poll and
-- the skill_review_updated event surface live progress), and `summary_json`
-- holds the summarizer's aggregated report when it completes.

CREATE TABLE skill_reviews (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    skill_name    TEXT NOT NULL,
    skill_source  TEXT NOT NULL DEFAULT 'library',   -- library | bundled
    status        TEXT NOT NULL DEFAULT 'running',    -- running | done | error | cancelled
    agent_mode    TEXT NOT NULL DEFAULT 'agents',     -- static | agents
    agents_json   TEXT NOT NULL DEFAULT '[]',
    static_json   TEXT,
    summary_json  TEXT,
    error         TEXT,
    created_by    TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_skill_reviews_ws ON skill_reviews(workspace_id, created_at);
