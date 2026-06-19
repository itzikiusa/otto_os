-- Agent Swarm: swarms (teams) of role-specialized agents that work projects
-- broken into tasks, coordinated by a per-swarm runtime. See
-- docs/superpowers/specs/2026-06-18-agent-swarm-design.md.
--
-- Conventions: ULID TEXT PKs, RFC3339 TEXT timestamps, JSON in *_json TEXT
-- columns, lowercase TEXT enums, every filter/join column indexed, every table
-- carries workspace_id.

-- A swarm: a named team/org within a workspace (NOT a "company").
CREATE TABLE swarms (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    preset_slug   TEXT,
    status        TEXT NOT NULL DEFAULT 'paused',   -- active | paused | aborted
    config_json   TEXT NOT NULL DEFAULT '{}',        -- {provider, model?, max_parallel_sessions, cwd_mode, default_soul?, auto_submit}
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_swarms_ws ON swarms(workspace_id);

-- A swarm member. The org tree is formed by reports_to (NULL = top, e.g. CEO).
CREATE TABLE swarm_agents (
    id             TEXT PRIMARY KEY,
    swarm_id       TEXT NOT NULL,
    workspace_id   TEXT NOT NULL,
    name           TEXT NOT NULL,
    title          TEXT NOT NULL DEFAULT '',         -- role label (CTO, Backend Dev, ...)
    reports_to     TEXT,                              -- parent agent id (org tree)
    provider       TEXT NOT NULL,                     -- claude | codex | agy
    model          TEXT,
    soul_name      TEXT,                              -- library soul ref (souls/<name>.md)
    soul_md        TEXT,                              -- inline persona (alternative to soul_name)
    specialization TEXT NOT NULL DEFAULT '',
    scope_md       TEXT NOT NULL DEFAULT '',          -- what they own / boundaries
    skills_json    TEXT NOT NULL DEFAULT '[]',        -- [{name, must_use}]
    schedule_json  TEXT,                              -- {cadence, every_min?, at?, weekday?, directive, enabled, last_run?}
    cwd_mode       TEXT,                              -- worktree | scratch | repo (override of swarm default)
    avatar         TEXT NOT NULL DEFAULT '',          -- emoji / initials
    status         TEXT NOT NULL DEFAULT 'active',    -- active | paused
    order_idx      INTEGER NOT NULL DEFAULT 0,
    created_by     TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX idx_swarm_agents_swarm ON swarm_agents(swarm_id);
CREATE INDEX idx_swarm_agents_ws ON swarm_agents(workspace_id);

-- A project within a swarm. Each project has its OWN kanban board (tasks).
CREATE TABLE swarm_projects (
    id            TEXT PRIMARY KEY,
    swarm_id      TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    name          TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    repo_path     TEXT,                               -- code projects (worktree base)
    goal_md       TEXT,                               -- high-level goal (planner input)
    status        TEXT NOT NULL DEFAULT 'active',     -- active | archived
    order_idx     INTEGER NOT NULL DEFAULT 0,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_swarm_projects_swarm ON swarm_projects(swarm_id);

-- A task: a kanban card. Belongs to a project; assigned to an agent; may depend
-- on other tasks (the DAG edges).
CREATE TABLE swarm_tasks (
    id                TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL,
    swarm_id          TEXT NOT NULL,
    workspace_id      TEXT NOT NULL,
    title             TEXT NOT NULL,
    description       TEXT NOT NULL DEFAULT '',
    assignee_agent_id TEXT,
    status            TEXT NOT NULL DEFAULT 'backlog', -- backlog|todo|in_progress|in_review|blocked|done|cancelled
    priority          TEXT NOT NULL DEFAULT 'medium',  -- low|medium|high|urgent
    parent_task_id    TEXT,                            -- goal ancestry / delegation parent
    depends_on_json   TEXT NOT NULL DEFAULT '[]',      -- [task_id] -> DAG edges
    labels_json       TEXT NOT NULL DEFAULT '[]',
    result_ref        TEXT,                            -- link/artifact when done
    delegated         INTEGER NOT NULL DEFAULT 0,      -- 1 once a leader has decomposed it
    order_idx         INTEGER NOT NULL DEFAULT 0,
    created_by        TEXT NOT NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL
);
CREATE INDEX idx_swarm_tasks_project ON swarm_tasks(project_id, status);
CREATE INDEX idx_swarm_tasks_swarm ON swarm_tasks(swarm_id);
CREATE INDEX idx_swarm_tasks_assignee ON swarm_tasks(assignee_agent_id);

-- A run: one execution of an agent against a task or a scheduled tick. Backs the
-- runs list / kanban-of-iterations and the run-graph.
CREATE TABLE swarm_runs (
    id            TEXT PRIMARY KEY,
    swarm_id      TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    project_id    TEXT,
    task_id       TEXT,
    agent_id      TEXT NOT NULL,
    session_id    TEXT,
    kind          TEXT NOT NULL DEFAULT 'task',        -- task|scheduled|review|handoff|planning|adhoc
    trigger       TEXT NOT NULL DEFAULT 'coordinator', -- manual|scheduled|coordinator|dependency|user
    status        TEXT NOT NULL DEFAULT 'queued',      -- queued|running|waiting|done|error|stopped
    attempt       INTEGER NOT NULL DEFAULT 0,
    summary       TEXT,
    result_json   TEXT,
    error         TEXT,
    tokens_input  INTEGER,
    tokens_output INTEGER,
    cost_usd      REAL,
    enqueued_at   TEXT NOT NULL,
    started_at    TEXT,
    finished_at   TEXT
);
CREATE INDEX idx_swarm_runs_swarm ON swarm_runs(swarm_id, status);
CREATE INDEX idx_swarm_runs_task ON swarm_runs(task_id);
CREATE INDEX idx_swarm_runs_agent ON swarm_runs(agent_id);
CREATE INDEX idx_swarm_runs_session ON swarm_runs(session_id);

-- The shared surface: a board where agents (and the user) post.
CREATE TABLE swarm_messages (
    id              TEXT PRIMARY KEY,
    swarm_id        TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    project_id      TEXT,
    task_id         TEXT,
    run_id          TEXT,
    author_agent_id TEXT,                              -- NULL = system/coordinator/user
    author_user_id  TEXT,                              -- set for human posts
    to_agent_id     TEXT,                              -- NULL = @all
    kind            TEXT NOT NULL DEFAULT 'message',   -- message|idea|review_request|review|decision|status|concern|escalation|handoff|system
    body            TEXT NOT NULL,
    meta_json       TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_swarm_messages_swarm ON swarm_messages(swarm_id, created_at);
CREATE INDEX idx_swarm_messages_project ON swarm_messages(project_id);
CREATE INDEX idx_swarm_messages_task ON swarm_messages(task_id);
