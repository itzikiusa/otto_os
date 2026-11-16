-- Add 'stopped' to the goal_loop_iterations.status CHECK constraint.
--
-- A Pause or Stop now closes the iteration in flight as 'stopped' instead of
-- leaving it 'executing' forever. The 0065 CHECK did not allow it, so this
-- widens it.
--
-- SQLite cannot ALTER a CHECK in place, so the table is rebuilt (create-new →
-- copy → drop → rename), keeping every column. Nothing references
-- goal_loop_iterations by foreign key, so the DROP cascades nowhere. The DDL
-- is the 0065 shape: nothing else has altered the table.
CREATE TABLE goal_loop_iterations_new (
    id              TEXT PRIMARY KEY,
    loop_id         TEXT NOT NULL,
    workspace_id    TEXT NOT NULL,
    idx             INTEGER NOT NULL,
    status          TEXT NOT NULL CHECK (status IN
        ('planning','executing','evaluating','digesting','done','error','stopped')),
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

INSERT INTO goal_loop_iterations_new
    (id, loop_id, workspace_id, idx, status, plan, agents_json, evaluation_json,
     context_in, context_out, tokens_input, tokens_output, cost_usd, started_at, finished_at)
SELECT
    id, loop_id, workspace_id, idx, status, plan, agents_json, evaluation_json,
    context_in, context_out, tokens_input, tokens_output, cost_usd, started_at, finished_at
FROM goal_loop_iterations;

DROP TABLE goal_loop_iterations;
ALTER TABLE goal_loop_iterations_new RENAME TO goal_loop_iterations;
CREATE INDEX idx_goal_loop_iters ON goal_loop_iterations(loop_id, idx);
CREATE INDEX idx_goal_loop_iters_ws ON goal_loop_iterations(workspace_id);
