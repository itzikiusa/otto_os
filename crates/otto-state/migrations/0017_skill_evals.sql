-- Skills Evaluator: test-and-improve a skill across scored iterations.
--
-- A `skill_evals` row is one run (a skill + task + validations); each round of
-- implement → validate → score → improve is a `skill_eval_iterations` row. The
-- per-validation live agent state + findings for an iteration live in that
-- iteration's `agents_json` (mirrors pr_reviews.agents_json), updated atomically
-- per array index during the run so the UI's poll surfaces live progress.

CREATE TABLE skill_evals (
    id                TEXT PRIMARY KEY,
    workspace_id      TEXT NOT NULL,
    source_skill      TEXT NOT NULL,
    task              TEXT NOT NULL,
    impl_cli          TEXT NOT NULL,
    target_iterations INTEGER NOT NULL DEFAULT 1,
    status            TEXT NOT NULL DEFAULT 'running',
    error             TEXT,
    summary           TEXT NOT NULL DEFAULT '',
    best_iteration    INTEGER,
    best_score        REAL,
    created_at        TEXT NOT NULL
);
CREATE INDEX idx_skill_evals_ws ON skill_evals(workspace_id, created_at);

CREATE TABLE skill_eval_iterations (
    id                  TEXT PRIMARY KEY,
    eval_id             TEXT NOT NULL REFERENCES skill_evals(id) ON DELETE CASCADE,
    iter                INTEGER NOT NULL,
    base_iter           INTEGER,
    skill_name          TEXT NOT NULL,
    skill_before        TEXT NOT NULL DEFAULT '',
    skill_after         TEXT,
    impl_provider       TEXT NOT NULL DEFAULT '',
    impl_session_id     TEXT,
    impl_summary        TEXT NOT NULL DEFAULT '',
    worktree_path       TEXT,
    status              TEXT NOT NULL DEFAULT 'pending',
    note                TEXT NOT NULL DEFAULT '',
    score               REAL NOT NULL DEFAULT 0,
    agents_json         TEXT NOT NULL DEFAULT '[]',
    improvement_summary TEXT NOT NULL DEFAULT '',
    skill_diff          TEXT NOT NULL DEFAULT '',
    created_at          TEXT NOT NULL
);
CREATE INDEX idx_skill_eval_iterations_eval ON skill_eval_iterations(eval_id, iter);
