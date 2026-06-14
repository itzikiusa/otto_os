-- API client ("Postman" section). Workspace-scoped collections, saved requests,
-- environments and execution history. Mirrors the otto_core::domain Api* structs.
-- ULID string PKs, UTC RFC3339 timestamps, free-form JSON in *_json TEXT columns.

-- Collections (and nested folders via parent_id; NULL = top-level).
CREATE TABLE api_collections (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    parent_id    TEXT REFERENCES api_collections(id) ON DELETE CASCADE,
    position     INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_api_collections_ws ON api_collections(workspace_id);
CREATE INDEX idx_api_collections_parent ON api_collections(parent_id);

-- Saved HTTP requests. A request optionally belongs to one collection; deleting
-- the collection drops its requests back to "ungrouped" (collection_id = NULL).
CREATE TABLE api_requests (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    collection_id TEXT REFERENCES api_collections(id) ON DELETE SET NULL,
    name          TEXT NOT NULL,
    method        TEXT NOT NULL,
    url           TEXT NOT NULL,
    headers_json  TEXT NOT NULL DEFAULT '[]',
    query_json    TEXT NOT NULL DEFAULT '[]',
    body_mode     TEXT NOT NULL DEFAULT 'none',
    body          TEXT NOT NULL DEFAULT '',
    auth_json     TEXT NOT NULL DEFAULT '{}',
    position      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_api_requests_ws ON api_requests(workspace_id);
CREATE INDEX idx_api_requests_collection ON api_requests(collection_id);

-- Named environments holding {{variable}} values. At most one row per workspace
-- has is_active = 1 (enforced in the repository's set_active()).
CREATE TABLE api_environments (
    id             TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    variables_json TEXT NOT NULL DEFAULT '{}',
    is_active      INTEGER NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL
);
CREATE INDEX idx_api_environments_ws ON api_environments(workspace_id);

-- Past executions + their response snapshots.
CREATE TABLE api_history (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    method        TEXT NOT NULL,
    url           TEXT NOT NULL,
    status        INTEGER,
    duration_ms   INTEGER,
    request_json  TEXT NOT NULL DEFAULT '{}',
    response_json TEXT NOT NULL DEFAULT '{}',
    executed_at   TEXT NOT NULL
);
CREATE INDEX idx_api_history_ws ON api_history(workspace_id, executed_at DESC);
