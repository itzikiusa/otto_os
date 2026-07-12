-- Vault v3 — the file-backed docs home. Files on disk are the source of truth;
-- these tables are a DERIVED index (rebuildable from a rescan at any time).
-- This migration also removes the Vault v2 embedding/code-intelligence tables:
-- embeddings and remote vector/graph backends are gone by design, and the
-- tree-sitter code index was replaced by agent-authored OKF docs. All dropped
-- tables held derived data only (re-creatable by re-indexing).

CREATE TABLE vaults (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    ws_id        TEXT NOT NULL,
    name         TEXT NOT NULL,
    root_path    TEXT NOT NULL,
    okf          INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL,
    last_scan_at TEXT,
    scan_state   TEXT NOT NULL DEFAULT 'idle',
    UNIQUE(ws_id, root_path)
);

CREATE TABLE vault_notes (
    vault_id         INTEGER NOT NULL,
    path             TEXT NOT NULL,           -- vault-relative, includes .md
    title            TEXT NOT NULL DEFAULT '',
    okf_type         TEXT,                    -- frontmatter `type` (OKF)
    description      TEXT,                    -- frontmatter `description`
    frontmatter_json TEXT NOT NULL DEFAULT '{}',
    tags_json        TEXT NOT NULL DEFAULT '[]',
    aliases_json     TEXT NOT NULL DEFAULT '[]',
    headings_json    TEXT NOT NULL DEFAULT '[]',
    word_count       INTEGER NOT NULL DEFAULT 0,
    size             INTEGER NOT NULL DEFAULT 0,
    mtime_ns         INTEGER NOT NULL DEFAULT 0,
    hash             TEXT NOT NULL DEFAULT '',
    reserved         INTEGER NOT NULL DEFAULT 0,  -- index.md / log.md (OKF)
    has_frontmatter  INTEGER NOT NULL DEFAULT 0,  -- a `---` YAML block exists
    parse_error      INTEGER NOT NULL DEFAULT 0,  -- fail-soft scan flag
    PRIMARY KEY (vault_id, path)
);

CREATE TABLE vault_links (
    vault_id   INTEGER NOT NULL,
    src_path   TEXT NOT NULL,
    raw_target TEXT NOT NULL,
    dst_path   TEXT,                          -- NULL = unresolved
    kind       TEXT NOT NULL DEFAULT 'wiki',  -- wiki | md | embed
    anchor     TEXT,
    alias      TEXT,
    pos        INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_vault_links_src ON vault_links(vault_id, src_path);
CREATE INDEX idx_vault_links_dst ON vault_links(vault_id, dst_path);

CREATE TABLE vault_tags (
    vault_id INTEGER NOT NULL,
    tag      TEXT NOT NULL,
    path     TEXT NOT NULL
);
CREATE INDEX idx_vault_tags_tag ON vault_tags(vault_id, tag);
CREATE INDEX idx_vault_tags_path ON vault_tags(vault_id, path);

-- Non-markdown files (attachments): explorer listing + embed/link resolution.
CREATE TABLE vault_files (
    vault_id INTEGER NOT NULL,
    path     TEXT NOT NULL,
    size     INTEGER NOT NULL DEFAULT 0,
    mtime_ns INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (vault_id, path)
);

-- (The vault_fts FTS5 table is created at runtime, like memories_fts, because
-- FTS5 availability depends on the linked SQLite.)

-- Vault v2 teardown (derived data only).
DROP TABLE IF EXISTS memory_vectors;
DROP TABLE IF EXISTS vault_backends;
DROP TABLE IF EXISTS code_symbols;
DROP TABLE IF EXISTS code_nodes;
DROP TABLE IF EXISTS code_edges;
DROP TABLE IF EXISTS code_repos;
