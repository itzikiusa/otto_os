CREATE TABLE network_profiles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    ssh_connection_id TEXT NOT NULL,
    endpoints_json TEXT NOT NULL,
    archived INTEGER NOT NULL DEFAULT 0,
    version INTEGER NOT NULL DEFAULT 1,
    created_by TEXT NOT NULL REFERENCES users(id),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX idx_network_profiles_workspace ON network_profiles(workspace_id, name);
