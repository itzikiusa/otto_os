-- Otto v1 schema. ULID string PKs, UTC RFC3339 timestamps, JSON in *_json columns.

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE,
    password_hash TEXT NOT NULL,
    display_name  TEXT NOT NULL DEFAULT '',
    is_root       INTEGER NOT NULL DEFAULT 0,
    disabled      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);

CREATE TABLE auth_sessions (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash   TEXT NOT NULL UNIQUE,
    created_at   TEXT NOT NULL,
    expires_at   TEXT NOT NULL,
    last_seen_at TEXT NOT NULL
);
CREATE INDEX idx_auth_sessions_user ON auth_sessions(user_id);

CREATE TABLE workspaces (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    root_path     TEXT NOT NULL,
    settings_json TEXT NOT NULL DEFAULT '{}',
    archived      INTEGER NOT NULL DEFAULT 0,
    created_at    TEXT NOT NULL
);

CREATE TABLE workspace_members (
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role         TEXT NOT NULL CHECK (role IN ('viewer','editor','admin')),
    PRIMARY KEY (workspace_id, user_id)
);
CREATE INDEX idx_ws_members_user ON workspace_members(user_id);

CREATE TABLE sessions (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    kind                TEXT NOT NULL CHECK (kind IN ('agent','connection')),
    provider            TEXT NOT NULL,
    title               TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN ('running','working','idle','exited','reconnectable')),
    cwd                 TEXT NOT NULL,
    provider_session_id TEXT,
    connection_id       TEXT REFERENCES connections(id) ON DELETE SET NULL,
    created_by          TEXT NOT NULL REFERENCES users(id),
    created_at          TEXT NOT NULL,
    last_active_at      TEXT NOT NULL,
    meta_json           TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX idx_sessions_ws_status ON sessions(workspace_id, status);

CREATE TABLE connections (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT REFERENCES workspaces(id) ON DELETE CASCADE,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL CHECK (kind IN ('ssh','mysql','redis','mongodb','clickhouse','custom')),
    params_json   TEXT NOT NULL DEFAULT '{}',
    secret_ref    TEXT,
    first_command TEXT,
    created_by    TEXT NOT NULL REFERENCES users(id),
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_connections_ws ON connections(workspace_id);

CREATE TABLE git_accounts (
    id           TEXT PRIMARY KEY,
    user_id      TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider     TEXT NOT NULL CHECK (provider IN ('github','bitbucket','gitlab')),
    label        TEXT NOT NULL,
    username     TEXT NOT NULL,
    token_ref    TEXT NOT NULL,
    api_base_url TEXT,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_git_accounts_user ON git_accounts(user_id);

CREATE TABLE repos (
    id             TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name           TEXT NOT NULL,
    path           TEXT NOT NULL,
    remote_url     TEXT,
    provider       TEXT CHECK (provider IN ('github','bitbucket','gitlab')),
    git_account_id TEXT REFERENCES git_accounts(id) ON DELETE SET NULL,
    created_at     TEXT NOT NULL
);
CREATE INDEX idx_repos_ws ON repos(workspace_id);

CREATE TABLE settings (
    key        TEXT PRIMARY KEY,
    value_json TEXT NOT NULL
);
