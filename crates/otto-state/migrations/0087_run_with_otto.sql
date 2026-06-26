-- Feature: Run with Otto — the one-button source→PR-draft pipeline. A source item
-- (Jira/Confluence/GitHub issue|PR/Slack|Telegram thread/Product task/Review finding/
-- Failing test/Scheduled-task report) is normalized into an `otto_runs` row whose
-- `status` IS the pipeline stage machine (otto_core::run::RunStatus), driven through
-- context→worktree→agent|goal-loop→proof→review→approval→PR-draft. Workspace-scoped.
-- Conventions: TEXT ULID ids, RFC3339 TEXT timestamps, *_json TEXT blobs, INTEGER
-- booleans, FK ON DELETE CASCADE. Server-set fields (branch/worktree/proof/review/pr)
-- are output-only — never written by the launch API.

CREATE TABLE otto_runs (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    title             TEXT NOT NULL,
    source_kind       TEXT NOT NULL,                       -- otto_core::run::SourceKind
    source_ref        TEXT NOT NULL,                       -- the handle (key/id/number/thread)
    source_url        TEXT,
    goal              TEXT NOT NULL DEFAULT '',            -- normalized task instruction
    mode              TEXT NOT NULL DEFAULT 'single_agent',-- single_agent | goal_loop
    provider          TEXT NOT NULL DEFAULT 'claude',
    repo_id           TEXT,                                -- resolved registered repo (NULL until resolved)
    repo_path         TEXT,
    base_branch       TEXT,
    branch            TEXT,                                -- otto-run/<id> or goal-loop/<id>
    worktree_path     TEXT,
    base_commit       TEXT,
    status            TEXT NOT NULL DEFAULT 'queued',      -- the stage machine
    error             TEXT,
    origin_kind       TEXT NOT NULL DEFAULT 'api',         -- slack|telegram|webhook|ui|mcp|api
    origin_chat       TEXT,
    origin_thread     TEXT,
    origin_user       TEXT,
    callback_url      TEXT,                                -- webhook origin: where to POST the result
    goal_loop_id      TEXT,
    review_id         TEXT,
    proof_pack_id     TEXT,
    proof_status      TEXT,                                -- snapshot for list view
    risk_score        INTEGER,                             -- snapshot 0..100
    findings_total    INTEGER NOT NULL DEFAULT 0,
    findings_blocking INTEGER NOT NULL DEFAULT 0,
    pr_draft_json     TEXT,                                -- {title,description,source_branch,target_branch}
    pr_url            TEXT,                                -- set only if a PR is actually opened
    auto_open_pr      INTEGER NOT NULL DEFAULT 0,
    approval_decision TEXT,                                -- approved | rejected
    approved_by       TEXT,
    approved_at       TEXT,
    result_summary    TEXT,
    context_summary   TEXT,                                -- truncated context packet (transparency)
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX idx_otto_runs_ws ON otto_runs(workspace_id, updated_at DESC);
CREATE INDEX idx_otto_runs_status ON otto_runs(status);
CREATE INDEX idx_otto_runs_thread ON otto_runs(workspace_id, origin_chat, origin_thread);

CREATE TABLE otto_run_events (
    id            TEXT PRIMARY KEY,
    run_id        TEXT NOT NULL REFERENCES otto_runs(id) ON DELETE CASCADE,
    workspace_id  TEXT NOT NULL,
    kind          TEXT NOT NULL,                           -- stage_enter|stage_ok|stage_error|note|approval|delivery
    status        TEXT,                                    -- run status at the time
    message       TEXT NOT NULL,
    detail_json   TEXT,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_otto_run_events_run ON otto_run_events(run_id, created_at);
