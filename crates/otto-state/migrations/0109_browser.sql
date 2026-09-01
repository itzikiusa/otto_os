-- 0109_browser.sql — Browser module: tabs + DOM annotations.
-- Annotations key on URL (not tab) so marks survive tab close and reattach.
CREATE TABLE browser_tabs (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    url          TEXT NOT NULL DEFAULT '',
    title        TEXT NOT NULL DEFAULT '',
    mode         TEXT NOT NULL DEFAULT 'reader', -- 'reader' | 'live'
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_browser_tabs_ws ON browser_tabs(workspace_id);

CREATE TABLE browser_annotations (
    id           TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL,
    tab_id       TEXT,
    url          TEXT NOT NULL,
    selector     TEXT NOT NULL,
    excerpt      TEXT NOT NULL DEFAULT '',
    text         TEXT NOT NULL DEFAULT '',
    comment      TEXT NOT NULL DEFAULT '',
    color        TEXT NOT NULL DEFAULT 'yellow',
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_browser_ann_ws_url ON browser_annotations(workspace_id, url);
