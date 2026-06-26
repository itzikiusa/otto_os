-- Feature: Scheduled Tasks — recurring, workspace-scoped jobs that run an agent
-- on a cadence (interval/daily/weekly), capture its output as a Markdown report,
-- and deliver it to a destination (Slack/Telegram/email/webhook). Driveable over
-- MCP. Conventions: TEXT ULID ids, RFC3339 TEXT timestamps, *_json TEXT blobs,
-- INTEGER booleans, FK ON DELETE CASCADE. `report_path` is output-only (set by the
-- engine, never by the API). Path segments are server-generated (task id + a server
-- timestamp), never the user-supplied name — see scheduled_tasks_engine::report_rel.

CREATE TABLE scheduled_tasks (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    kind            TEXT NOT NULL DEFAULT 'agent_prompt',   -- agent_prompt
    prompt          TEXT NOT NULL DEFAULT '',               -- agent instructions
    skill           TEXT,                                   -- optional skill slug to inline
    provider        TEXT NOT NULL DEFAULT 'claude',         -- v1: claude only
    model           TEXT NOT NULL DEFAULT '',               -- '' => provider default
    cwd             TEXT NOT NULL DEFAULT '',               -- '' => per-task scratch dir
    schedule_json   TEXT NOT NULL DEFAULT '{}',             -- {cadence,every_min,at,weekday}
    destination_json TEXT NOT NULL DEFAULT '{}',            -- {type, ...}
    enabled         INTEGER NOT NULL DEFAULT 1,
    last_run_at     TEXT,                                   -- cursor (advanced on run completion)
    last_status     TEXT,                                   -- ok | error
    next_run_at     TEXT,                                   -- computed for display
    created_by      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_scheduled_tasks_ws ON scheduled_tasks(workspace_id, enabled);
CREATE INDEX idx_scheduled_tasks_enabled ON scheduled_tasks(enabled);

CREATE TABLE scheduled_task_runs (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES scheduled_tasks(id) ON DELETE CASCADE,
    workspace_id    TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'running',        -- running | ok | error
    trigger         TEXT NOT NULL DEFAULT 'schedule',       -- schedule | manual
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    summary         TEXT NOT NULL DEFAULT '',
    report_path     TEXT,                                   -- absolute path on disk (output-only)
    report_rel      TEXT,                                   -- relative name for serving
    delivered       INTEGER NOT NULL DEFAULT 0,
    delivery_error  TEXT,
    error           TEXT,                                   -- run error when status=error
    session_id      TEXT,                                   -- reserved (v1 run is headless)
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_str_task ON scheduled_task_runs(task_id, started_at DESC);
CREATE INDEX idx_str_ws ON scheduled_task_runs(workspace_id, started_at DESC);
