-- Eval Lab: golden tasks (per-repo corpus), provider×skill×prompt matrices, and
-- multi-signal scoring / proof / matrix columns on the existing eval tables.
-- Append-only; new columns are nullable or NOT NULL DEFAULT <const> (precedent:
-- 0019_skill_eval_config.sql) so existing rows and installs are unaffected.

-- Reusable, per-repo evaluation tasks (golden corpus + regression cases).
CREATE TABLE eval_golden_tasks (
  id             TEXT PRIMARY KEY,
  workspace_id   TEXT NOT NULL,
  repo_key       TEXT NOT NULL,                 -- registered repo id, else workspace id
  name           TEXT NOT NULL,
  prompt         TEXT NOT NULL,
  skill          TEXT NOT NULL DEFAULT '',
  test_cmd       TEXT NOT NULL DEFAULT '',
  lint_cmd       TEXT NOT NULL DEFAULT '',
  build_cmd      TEXT NOT NULL DEFAULT '',
  rubric         TEXT NOT NULL DEFAULT '',
  tags_json      TEXT NOT NULL DEFAULT '[]',
  origin         TEXT NOT NULL DEFAULT 'manual', -- 'manual' | 'regression'
  source_eval_id TEXT,                           -- set when origin='regression'
  source_iter_id TEXT,
  enabled        INTEGER NOT NULL DEFAULT 1,
  created_by     TEXT NOT NULL,
  created_at     TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);
CREATE INDEX idx_golden_ws_repo ON eval_golden_tasks(workspace_id, repo_key);
-- One regression case per source iteration (dedupe failed-eval → regression).
CREATE UNIQUE INDEX idx_golden_src_iter
  ON eval_golden_tasks(source_iter_id) WHERE source_iter_id IS NOT NULL;

-- Provider × skill × prompt comparison runs. Cells are skill_evals rows that
-- carry this matrix_id + the dim_* discriminators.
CREATE TABLE eval_matrices (
  id             TEXT PRIMARY KEY,
  workspace_id   TEXT NOT NULL,
  name           TEXT NOT NULL,
  status         TEXT NOT NULL DEFAULT 'running', -- running|done|error|cancelled
  repo_key       TEXT NOT NULL DEFAULT '',
  mode           TEXT NOT NULL DEFAULT 'generate',
  providers_json TEXT NOT NULL DEFAULT '[]',
  skills_json    TEXT NOT NULL DEFAULT '[]',
  prompts_json   TEXT NOT NULL DEFAULT '[]',      -- [{label,task,golden_task_id?}]
  created_by     TEXT NOT NULL,
  created_at     TEXT NOT NULL
);
CREATE INDEX idx_matrix_ws ON eval_matrices(workspace_id);

-- Eval-run: run mode, golden link, matrix membership + dimensions, headline
-- composite, promotion state.
ALTER TABLE skill_evals ADD COLUMN mode TEXT NOT NULL DEFAULT 'generate';
ALTER TABLE skill_evals ADD COLUMN golden_task_id TEXT;
ALTER TABLE skill_evals ADD COLUMN matrix_id TEXT;
ALTER TABLE skill_evals ADD COLUMN dim_provider TEXT;
ALTER TABLE skill_evals ADD COLUMN dim_skill TEXT;
ALTER TABLE skill_evals ADD COLUMN dim_prompt TEXT;
ALTER TABLE skill_evals ADD COLUMN composite_score REAL;
ALTER TABLE skill_evals ADD COLUMN promoted INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skill_evals ADD COLUMN promoted_at TEXT;
ALTER TABLE skill_evals ADD COLUMN promoted_by TEXT;
CREATE INDEX idx_skill_evals_matrix ON skill_evals(matrix_id);

-- Iteration: multi-signal score JSON, its proof pack, and the human rating.
ALTER TABLE skill_eval_iterations ADD COLUMN scoring_json TEXT;
ALTER TABLE skill_eval_iterations ADD COLUMN proof_pack_id TEXT;
ALTER TABLE skill_eval_iterations ADD COLUMN human_rating INTEGER;
ALTER TABLE skill_eval_iterations ADD COLUMN human_note TEXT;
ALTER TABLE skill_eval_iterations ADD COLUMN human_rater TEXT;
