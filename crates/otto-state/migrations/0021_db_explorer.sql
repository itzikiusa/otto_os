-- DB Explorer: saved queries, query history, and Superset-like dashboards.

CREATE TABLE db_saved_queries (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    connection_id TEXT,
    name          TEXT NOT NULL,
    statement     TEXT NOT NULL,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_db_saved_ws ON db_saved_queries(workspace_id);

CREATE TABLE db_query_history (
    id            TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL,
    statement     TEXT NOT NULL,
    ok            INTEGER NOT NULL,
    duration_ms   INTEGER NOT NULL DEFAULT 0,
    row_count     INTEGER NOT NULL DEFAULT 0,
    error         TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_db_hist_conn ON db_query_history(connection_id, created_at DESC);

CREATE TABLE db_dashboards (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    name          TEXT NOT NULL,
    layout_json   TEXT NOT NULL DEFAULT '[]',
    refresh_secs  INTEGER,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_db_dash_ws ON db_dashboards(workspace_id);

CREATE TABLE db_widgets (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    dashboard_id  TEXT,
    connection_id TEXT NOT NULL,
    title         TEXT NOT NULL,
    statement     TEXT NOT NULL,
    viz           TEXT NOT NULL DEFAULT 'table',
    mapping_json  TEXT NOT NULL DEFAULT '{}',
    options_json  TEXT NOT NULL DEFAULT '{}',
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_db_widget_ws ON db_widgets(workspace_id);
CREATE INDEX idx_db_widget_dash ON db_widgets(dashboard_id);
