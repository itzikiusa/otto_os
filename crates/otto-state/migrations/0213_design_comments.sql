-- Design Hall — review comments on a design (docs/features/design-hall.md
-- § Comments; `otto_design::comments`).
--
-- ADDITIVE ONLY: one new table + its indexes; no existing table is touched,
-- so the previous build boots on this schema unchanged (it never reads it).
--
-- A comment belongs to an artifact AND to the version it was made on
-- (`version_id`; `version_seq` is kept beside it so "made on v3" still reads
-- after retention prunes that version). Replies are one level deep
-- (`parent_id` → a top-level comment) and inherit the thread's version; only
-- a top-level comment carries a pin or a resolved state. The optional pin is
-- a point on the rendered stage as FRACTIONS of its content box (0..1, so it
-- survives a resize or a device-frame change), plus an optional frame/page id
-- for multi-frame formats.
--
-- No foreign-key clauses (the design-graph convention): a hard delete of the
-- artifact removes its comments explicitly (`Store::delete_artifact`);
-- archiving keeps them.
CREATE TABLE IF NOT EXISTS design_comments (
    id               TEXT PRIMARY KEY,
    artifact_id      TEXT NOT NULL,              -- design_artifacts.id
    workspace_id     TEXT NOT NULL,              -- the artifact's (RBAC axis)
    version_id       TEXT,                       -- design_versions.id; NULL = no version yet
    version_seq      INTEGER,                    -- that version's seq
    parent_id        TEXT,                       -- design_comments.id (one level of replies)
    author_kind      TEXT NOT NULL DEFAULT 'user', -- user | agent
    author_id        TEXT NOT NULL,              -- users.id
    author_name      TEXT NOT NULL DEFAULT '',   -- display name at write time
    session_id       TEXT,                       -- the agent session (author_kind = agent)
    body             TEXT NOT NULL,
    pin_x            REAL,                       -- 0..1 of the stage's content box
    pin_y            REAL,
    pin_frame        TEXT,                       -- frame / page id, multi-frame formats
    resolved         INTEGER NOT NULL DEFAULT 0,
    resolved_by      TEXT,
    resolved_by_name TEXT,
    resolved_at      TEXT,
    edited_at        TEXT,                       -- last body edit
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_design_comments_artifact ON design_comments(artifact_id, created_at);
CREATE INDEX IF NOT EXISTS idx_design_comments_parent ON design_comments(parent_id);
