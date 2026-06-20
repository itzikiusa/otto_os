-- Saved workspace work-queue views for the Mission Control surface (B4).
-- Each row is one user-defined filter on the 6-bucket view
-- (needs_you / working / review_ready / waiting / failed / budget_warn).
CREATE TABLE saved_views (
    id          TEXT NOT NULL PRIMARY KEY,
    user_id     TEXT NOT NULL,
    workspace_id TEXT NOT NULL,
    name        TEXT NOT NULL,
    filter_json TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL
);

CREATE INDEX idx_saved_views_workspace ON saved_views(workspace_id);
CREATE INDEX idx_saved_views_user ON saved_views(user_id, workspace_id);
