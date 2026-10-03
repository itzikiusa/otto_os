-- Workbench: per-user scratch files ("workbench docs") with a FULL, append-only
-- edit history. Additive only.
--
-- History is removed ONLY by an explicit permanent delete of a trashed doc
-- (`WorkbenchRepo::purge`): no foreign key cascades into these tables and no
-- retention job lists them, so neither a workspace/user delete nor the hourly
-- pruner can drop a revision behind the user's back.
--
-- Content is content-addressed: `workbench_blobs` holds each distinct text
-- once (sha256 hex key); docs and revisions reference it by hash, so a doc
-- restored to an older state, or two identical scripts, cost one blob.
CREATE TABLE IF NOT EXISTS workbench_blobs (
    hash        TEXT PRIMARY KEY,
    content     TEXT NOT NULL,
    size        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS workbench_docs (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    owner_id      TEXT NOT NULL,
    name          TEXT NOT NULL,
    language      TEXT NOT NULL DEFAULT 'auto',
    pinned        INTEGER NOT NULL DEFAULT 0,
    folder        TEXT NOT NULL DEFAULT '',
    tags_json     TEXT NOT NULL DEFAULT '[]',
    content_hash  TEXT NOT NULL,
    size          INTEGER NOT NULL DEFAULT 0,
    rev           INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    deleted_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_workbench_docs_owner
    ON workbench_docs(workspace_id, owner_id, deleted_at, updated_at);

CREATE TABLE IF NOT EXISTS workbench_revisions (
    doc_id         TEXT NOT NULL,
    seq            INTEGER NOT NULL,
    kind           TEXT NOT NULL,
    content_hash   TEXT NOT NULL,
    size           INTEGER NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    saves          INTEGER NOT NULL DEFAULT 1,
    restored_from  INTEGER,
    PRIMARY KEY (doc_id, seq)
);
CREATE INDEX IF NOT EXISTS idx_workbench_revisions_hash
    ON workbench_revisions(content_hash);

CREATE TABLE IF NOT EXISTS workbench_assets (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    owner_id      TEXT NOT NULL,
    mime          TEXT NOT NULL,
    size          INTEGER NOT NULL,
    sha256        TEXT NOT NULL,
    data          BLOB NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_workbench_assets_owner
    ON workbench_assets(workspace_id, owner_id);
