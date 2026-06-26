-- Proof Packs: the evidence layer. Every meaningful unit of agent work carries a
-- pack whose status is DERIVED from its evidence artifacts, not claimed.
--
-- Conventions (matching the swarm/goal-loops tables): ULID TEXT PKs, RFC3339 TEXT
-- timestamps, JSON in *_json TEXT columns, lowercase TEXT enums, every filter/join
-- column indexed, every table carries workspace_id.

CREATE TABLE proof_packs (
    id             TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL,
    work_item_kind TEXT NOT NULL,                  -- session | goal_loop | review | workflow_run | task | manual
    work_item_id   TEXT NOT NULL,
    title          TEXT NOT NULL DEFAULT '',
    status         TEXT NOT NULL DEFAULT 'missing', -- missing | partial | passed | failed | waived
    summary        TEXT NOT NULL DEFAULT '',
    risk_score     INTEGER NOT NULL DEFAULT 0,      -- 0..100, derived
    parent_pack_id TEXT,                            -- optional rollup parent
    waived_by      TEXT,                            -- user id, set when status=waived
    waived_reason  TEXT,
    created_by     TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
-- Exactly one pack per work item (the ensure-or-create gate key).
CREATE UNIQUE INDEX idx_proof_packs_workitem ON proof_packs(work_item_kind, work_item_id);
CREATE INDEX idx_proof_packs_ws ON proof_packs(workspace_id, status);
CREATE INDEX idx_proof_packs_parent ON proof_packs(parent_pack_id);

CREATE TABLE proof_artifacts (
    id             TEXT PRIMARY KEY,
    proof_pack_id  TEXT NOT NULL REFERENCES proof_packs(id) ON DELETE CASCADE,
    workspace_id   TEXT NOT NULL,
    kind           TEXT NOT NULL,                  -- command|log|screenshot|diff|ci|api|db|review|approval|self_review
    title          TEXT NOT NULL,
    content_ref    TEXT,                           -- inline text (capped), URL, or file ref; ref_kind in metadata
    status         TEXT NOT NULL DEFAULT 'info',   -- passed | failed | pending | info
    metadata_json  TEXT NOT NULL DEFAULT '{}',
    created_by     TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL
);
CREATE INDEX idx_proof_artifacts_pack ON proof_artifacts(proof_pack_id, created_at);
