-- Vault / Context Engine v2 — "Repo Brain".
--
-- Append-only migration. Adds the code-intelligence layer (a persistent
-- tree-sitter symbol index + a typed code dependency graph), per-repo index
-- state, and per-workspace remote-backend configuration (Qdrant / SurrealDB /
-- Ollama). Nothing here touches existing tables.
--
-- The FTS5 keyword index is created at RUNTIME (see otto-memory `ensure_fts`),
-- not here: a `CREATE VIRTUAL TABLE` would abort the whole migration on a SQLite
-- build without FTS5, bricking the daemon. Runtime creation degrades to the
-- existing LIKE search instead.

-- Per-repo index state: one row per (workspace, repo root) we have indexed.
CREATE TABLE code_repos (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    root         TEXT NOT NULL,                 -- absolute path on disk
    name         TEXT NOT NULL,                 -- display name (basename)
    head         TEXT,                          -- git HEAD oid at last index
    files        INTEGER NOT NULL DEFAULT 0,
    symbols      INTEGER NOT NULL DEFAULT 0,
    edges        INTEGER NOT NULL DEFAULT 0,
    chunks       INTEGER NOT NULL DEFAULT 0,    -- embedded code chunks
    status       TEXT NOT NULL DEFAULT 'idle',  -- idle|indexing|ready|error
    message      TEXT,
    indexed_at   TEXT,
    created_at   TEXT NOT NULL,
    UNIQUE(workspace_id, root)
);
CREATE INDEX idx_code_repos_ws ON code_repos(workspace_id);

-- Persistent tree-sitter symbol index.
CREATE TABLE code_symbols (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    repo_id      TEXT NOT NULL REFERENCES code_repos(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    kind         TEXT NOT NULL,                 -- function|struct|class|method|type|interface|...
    lang         TEXT NOT NULL,                 -- rs|go|ts|py|...
    file         TEXT NOT NULL,                 -- repo-relative path
    line         INTEGER NOT NULL,              -- 1-based
    signature    TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_code_symbols_repo ON code_symbols(repo_id);
CREATE INDEX idx_code_symbols_name ON code_symbols(workspace_id, name);
CREATE INDEX idx_code_symbols_file ON code_symbols(repo_id, file);

-- Code dependency / knowledge graph: nodes + typed edges. Files, symbols,
-- external services, DB tables, endpoints and generated docs are all nodes; the
-- edges carry the relation (calls / imports / http_call / db_call / test_of /
-- documents / defined_in / depends_on).
CREATE TABLE code_nodes (
    id           TEXT PRIMARY KEY,              -- stable: hash(workspace,repo,kind,key)
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    repo_id      TEXT,                          -- nullable for external/service nodes
    kind         TEXT NOT NULL,                 -- file|symbol|service|db_table|endpoint|doc|external
    key          TEXT NOT NULL,                 -- file path / symbol name / service name / table
    label        TEXT NOT NULL,
    file         TEXT,
    line         INTEGER,
    meta_json    TEXT NOT NULL DEFAULT '{}',
    created_at   TEXT NOT NULL,
    UNIQUE(workspace_id, repo_id, kind, key)
);
CREATE INDEX idx_code_nodes_repo ON code_nodes(repo_id);
CREATE INDEX idx_code_nodes_ws_kind ON code_nodes(workspace_id, kind);

CREATE TABLE code_edges (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    repo_id      TEXT,
    src_id       TEXT NOT NULL REFERENCES code_nodes(id) ON DELETE CASCADE,
    dst_id       TEXT NOT NULL REFERENCES code_nodes(id) ON DELETE CASCADE,
    rel          TEXT NOT NULL,                 -- calls|imports|http_call|db_call|test_of|documents|defined_in|depends_on
    detail       TEXT NOT NULL DEFAULT '',      -- "GET /limits" | "SELECT ... FROM limits"
    weight       REAL NOT NULL DEFAULT 1.0,
    file         TEXT,
    line         INTEGER,
    created_at   TEXT NOT NULL,
    UNIQUE(workspace_id, src_id, dst_id, rel, detail)
);
CREATE INDEX idx_code_edges_src ON code_edges(src_id);
CREATE INDEX idx_code_edges_dst ON code_edges(dst_id);
CREATE INDEX idx_code_edges_repo ON code_edges(repo_id);

-- Per-workspace remote backend configuration. Secrets (api keys / passwords) are
-- stored in the Keychain by reference; only non-secret connection config lives
-- here. `role` records which layer this backend serves.
CREATE TABLE vault_backends (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    kind         TEXT NOT NULL,                 -- qdrant|surreal|ollama
    enabled      INTEGER NOT NULL DEFAULT 0,
    url          TEXT NOT NULL DEFAULT '',
    role         TEXT NOT NULL DEFAULT '',      -- vector|graph|embed
    config_json  TEXT NOT NULL DEFAULT '{}',
    status       TEXT NOT NULL DEFAULT 'unknown', -- unknown|ok|error|installing
    message      TEXT,
    updated_at   TEXT NOT NULL,
    UNIQUE(workspace_id, kind)
);
CREATE INDEX idx_vault_backends_ws ON vault_backends(workspace_id);
