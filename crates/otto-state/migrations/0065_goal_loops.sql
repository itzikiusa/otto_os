-- Goal Loops: a bounded, goal-directed multi-agent iteration engine.
-- A loop runs Plan -> Execute -> Evaluate -> Digest cycles toward a goal with
-- machine-checked acceptance criteria, on an isolated git branch, until the goal
-- is met or a hard limit (iterations / active time) is hit.
--
-- Conventions follow the swarm tables (0029): workspace_id / created_by are plain
-- TEXT (no FK clauses); cleanup of child rows is done explicitly by the repo.

CREATE TABLE goal_loops (
    id                  TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL,
    name                TEXT NOT NULL,
    repo_path           TEXT NOT NULL,
    definition_json     TEXT NOT NULL,
    limits_json         TEXT NOT NULL,
    config_json         TEXT NOT NULL,
    status              TEXT NOT NULL CHECK (status IN
        ('draft','running','paused','blocked','succeeded','exhausted','failed','stopped')),
    phase               TEXT NOT NULL DEFAULT 'done',
    iterations_started  INTEGER NOT NULL DEFAULT 0,
    current_iteration   INTEGER NOT NULL DEFAULT 0,
    progress_pct        INTEGER NOT NULL DEFAULT 0,
    context_digest      TEXT NOT NULL DEFAULT '',
    branch              TEXT,
    worktree_path       TEXT,
    base_commit         TEXT,
    summary             TEXT,
    error               TEXT,
    run_started_at      TEXT,
    elapsed_secs        INTEGER NOT NULL DEFAULT 0,
    cost_usd            REAL NOT NULL DEFAULT 0,
    created_by          TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    finished_at         TEXT
);
CREATE INDEX idx_goal_loops_ws ON goal_loops(workspace_id, status);

CREATE TABLE goal_loop_iterations (
    id              TEXT PRIMARY KEY,
    loop_id         TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    idx             INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN
        ('planning','executing','evaluating','digesting','done','error')),
    plan            TEXT NOT NULL DEFAULT '',
    agents_json     TEXT NOT NULL DEFAULT '[]',
    evaluation_json TEXT,
    context_in      TEXT NOT NULL DEFAULT '',
    context_out     TEXT NOT NULL DEFAULT '',
    tokens_input    INTEGER NOT NULL DEFAULT 0,
    tokens_output   INTEGER NOT NULL DEFAULT 0,
    cost_usd        REAL NOT NULL DEFAULT 0,
    started_at      TEXT NOT NULL,
    finished_at     TEXT,
    UNIQUE(loop_id, idx)
);
CREATE INDEX idx_goal_loop_iters ON goal_loop_iterations(loop_id, idx);
CREATE INDEX idx_goal_loop_iters_ws ON goal_loop_iterations(workspace_id);
