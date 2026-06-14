-- API client AUTOMATIONS (collection runner). A workspace-scoped, ordered
-- sequence of saved-request executions with optional per-step assertions and
-- variable extraction (chained across steps). ULID string PK, UTC RFC3339
-- timestamps, the step list stored as free-form JSON in steps_json TEXT.

CREATE TABLE api_automations (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    -- [{ "request_id", "assertions": [{ "kind", "path"?, "op", "value" }],
    --    "extract": [{ "path", "var" }] }]
    steps_json   TEXT NOT NULL DEFAULT '[]',
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
CREATE INDEX idx_api_automations_ws ON api_automations(workspace_id);
