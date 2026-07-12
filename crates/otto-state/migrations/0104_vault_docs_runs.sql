-- Vault docs-agent runs — the durable mirror of the in-memory run registry.
-- One row per docs run AND per refine turn (kind); `payload` carries the full
-- VaultDocsRun JSON snapshot (few KB) and is the source of truth for detail
-- rendering; the flat columns exist for listing/filtering only. Any row still
-- non-terminal at daemon startup was interrupted by the restart — the startup
-- sweep flips it to 'interrupted' and trashes its orphaned _drafts dir.
-- No CHECK constraints by design (0058 lesson: an old CHECK blocks new kinds).

CREATE TABLE vault_docs_runs (
    id          TEXT PRIMARY KEY,              -- run id (docs) / turn id (refine)
    vault_id    INTEGER NOT NULL,
    ws_id       TEXT NOT NULL,
    kind        TEXT NOT NULL DEFAULT 'docs',  -- docs | refine
    state       TEXT NOT NULL,                 -- running|summarizing|done|error|cancelled|interrupted
    prompt      TEXT NOT NULL DEFAULT '',
    target_dir  TEXT NOT NULL DEFAULT '',
    note_path   TEXT NOT NULL DEFAULT '',      -- refine: the note being edited
    payload     TEXT NOT NULL DEFAULT '{}',    -- full VaultDocsRun JSON snapshot
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    updated_at  TEXT NOT NULL
);
CREATE INDEX idx_vault_docs_runs_vault ON vault_docs_runs(vault_id, started_at DESC);
CREATE INDEX idx_vault_docs_runs_state ON vault_docs_runs(state);
