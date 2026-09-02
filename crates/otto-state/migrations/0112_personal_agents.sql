-- Feature: Personal Agents — named, persistent grok-bot-style agents with a
-- persona (soul_md), a pinned provider+model, 1..N schedules (each with its own
-- cursor), per-agent delivery, run history mirroring scheduled_task_runs, and
-- inter-agent rooms whose every message is persisted and user-visible.
-- Conventions: TEXT ULID ids, RFC3339 TEXT timestamps, *_json TEXT blobs,
-- INTEGER booleans, FK ON DELETE CASCADE. `report_path` is output-only (set by
-- the engine, never by the API); report path segments are server-generated
-- (agent id + a server timestamp), never the user-supplied name.

CREATE TABLE personal_agents (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    avatar          TEXT NOT NULL DEFAULT '',               -- emoji / short glyph
    soul_md         TEXT NOT NULL DEFAULT '',               -- persona, materialized into cwd CLAUDE.md/AGENTS.md
    provider        TEXT NOT NULL DEFAULT 'claude',
    model           TEXT NOT NULL DEFAULT '',               -- '' => provider default
    cwd             TEXT NOT NULL DEFAULT '',               -- '' => data_dir/personal/<agent-id>/
    browser         INTEGER NOT NULL DEFAULT 0,             -- reconcile otto-browser MCP into runs
    delivery_json   TEXT NOT NULL DEFAULT '{}',             -- {type: none|slack|telegram|email|webhook, ...}
    enabled         INTEGER NOT NULL DEFAULT 1,
    chat_session_id TEXT,                                   -- the ONE interactive chat session (output-only)
    created_by      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_personal_agents_ws ON personal_agents(workspace_id, enabled);

-- 1..N schedules per agent; each schedule carries its OWN cursor
-- (last_run_at/next_run_at) so "daily recap 09:00" + "15-min needs-attention"
-- coexist on one agent without fighting over a shared cursor.
CREATE TABLE personal_agent_schedules (
    id              TEXT PRIMARY KEY,
    agent_id        TEXT NOT NULL REFERENCES personal_agents(id) ON DELETE CASCADE,
    schedule_json   TEXT NOT NULL DEFAULT '{}',             -- existing cadence format {cadence,every_min,at,weekday,expr}
    timezone        TEXT NOT NULL DEFAULT 'UTC',            -- IANA tz for daily/weekly/cron
    directive       TEXT NOT NULL DEFAULT '',               -- the run's task prompt
    enabled         INTEGER NOT NULL DEFAULT 1,
    last_run_at     TEXT,                                   -- per-schedule cursor (advanced on run completion)
    next_run_at     TEXT,                                   -- computed for display
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_pa_schedules_agent ON personal_agent_schedules(agent_id, enabled);

-- Mirrors scheduled_task_runs (+ schedule_id; manual runs may have none).
CREATE TABLE personal_agent_runs (
    id              TEXT PRIMARY KEY,
    agent_id        TEXT NOT NULL REFERENCES personal_agents(id) ON DELETE CASCADE,
    schedule_id     TEXT,                                   -- NULL for manual runs without a schedule
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
    session_id      TEXT,                                   -- the visible agent session driving the run
    report_hash     TEXT,                                   -- notify-on-change content hash
    attempts        INTEGER NOT NULL DEFAULT 1,
    skipped_delivery INTEGER NOT NULL DEFAULT 0,            -- notify_on_change: unchanged => skipped
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_par_agent ON personal_agent_runs(agent_id, started_at DESC);
CREATE INDEX idx_par_ws ON personal_agent_runs(workspace_id, started_at DESC);

-- Rooms: the ONLY agent-to-agent transport; every message is persisted and
-- rendered to the user. author_kind 'agent' => author_id is a personal_agents
-- id; 'user' => author_id is a users id.
CREATE TABLE agent_rooms (
    id              TEXT PRIMARY KEY,
    workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    name            TEXT NOT NULL,
    created_by      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_agent_rooms_ws ON agent_rooms(workspace_id);

CREATE TABLE agent_room_members (
    room_id         TEXT NOT NULL REFERENCES agent_rooms(id) ON DELETE CASCADE,
    agent_id        TEXT NOT NULL REFERENCES personal_agents(id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (room_id, agent_id)
);

CREATE TABLE agent_room_messages (
    id              TEXT PRIMARY KEY,                       -- ULID: lexicographic order == time order (paging key)
    room_id         TEXT NOT NULL REFERENCES agent_rooms(id) ON DELETE CASCADE,
    author_kind     TEXT NOT NULL CHECK (author_kind IN ('agent','user')),
    author_id       TEXT NOT NULL,
    text            TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_arm_room ON agent_room_messages(room_id, id);
