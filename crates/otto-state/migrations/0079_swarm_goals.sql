-- Goals the leader verifies for a task/project, plus swarm-level "standing"
-- (always-applied) goal templates. The verification controller checks each goal
-- sequentially: pass → next; a blocking miss → fix-request to the dev → re-verify
-- (more thoroughly) up to `max_retries`, then warn (close) or unmet (far).
CREATE TABLE swarm_goals (
    id            TEXT PRIMARY KEY,
    swarm_id      TEXT NOT NULL,
    workspace_id  TEXT NOT NULL,
    project_id    TEXT,                              -- project-scoped goal (NULL ok)
    task_id       TEXT,                              -- task-scoped goal; both NULL + kind=standing = swarm template
    kind          TEXT NOT NULL DEFAULT 'explicit',  -- explicit | standing
    title         TEXT NOT NULL,
    description   TEXT NOT NULL DEFAULT '',
    metric        TEXT,                              -- e.g. 'runtime_seconds' (optional, metric goals)
    comparator    TEXT,                              -- lte|gte|eq|contains|absent (optional)
    target_value  REAL,                              -- pass threshold (e.g. 120 = under 2 min)
    block_value   REAL,                              -- worse than this => blocker; else warn band
    verify_cmd    TEXT,                              -- optional ground-truth command (run in the worktree)
    max_retries   INTEGER NOT NULL DEFAULT 3,        -- per-goal fix attempts before "could not be achieved"
    blocking      INTEGER NOT NULL DEFAULT 1,        -- 0 = advisory (never fix-loops, never blocks merge)
    status        TEXT NOT NULL DEFAULT 'pending',   -- pending|verifying|passed|warned|unmet|skipped|error
    verdict_json  TEXT,                              -- last full leader verdict
    iterations    INTEGER NOT NULL DEFAULT 0,        -- fix attempts taken so far (restart-safe)
    order_idx     INTEGER NOT NULL DEFAULT 0,
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);
CREATE INDEX idx_swarm_goals_swarm ON swarm_goals(swarm_id, kind);
CREATE INDEX idx_swarm_goals_project ON swarm_goals(project_id);
CREATE INDEX idx_swarm_goals_task ON swarm_goals(task_id);
