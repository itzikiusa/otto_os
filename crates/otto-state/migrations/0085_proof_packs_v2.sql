-- Proof Packs v2 — "the trust layer".
-- Additive only: new columns + two new tables. Existing v1 rows keep working
-- (every new column has a default / is nullable). FKs reference existing real
-- tables (proof_packs / proof_artifacts / repos).

-- Pack: graded done-contract score, repo + PR link, waiver timestamp.
ALTER TABLE proof_packs ADD COLUMN done_score INTEGER NOT NULL DEFAULT 0; -- 0..100, derived
ALTER TABLE proof_packs ADD COLUMN repo_id    TEXT;                       -- registered repo this work touched
ALTER TABLE proof_packs ADD COLUMN pr_number  INTEGER;                    -- set once PR-linked (CI refresh / report)
ALTER TABLE proof_packs ADD COLUMN waived_at  TEXT;                       -- RFC3339, set alongside waived_by

-- Artifact integrity: SHA-256 of the full stored content at write time.
ALTER TABLE proof_artifacts ADD COLUMN content_sha256 TEXT;

-- Per-repo proof requirements (RepoProofConfig JSON). Default '{}' = v1 behavior.
ALTER TABLE repos ADD COLUMN proof_config_json TEXT NOT NULL DEFAULT '{}';

-- Immutable snapshots: a frozen, tamper-evident capture of a pack's evidence
-- plus rendered Markdown/HTML reports. Append-only (never UPDATEd/DELETEd except
-- the cascade when the owning pack is removed).
CREATE TABLE proof_snapshots (
    id            TEXT PRIMARY KEY,
    proof_pack_id TEXT NOT NULL REFERENCES proof_packs(id) ON DELETE CASCADE,
    workspace_id  TEXT NOT NULL,
    seq           INTEGER NOT NULL,                 -- 1,2,3… per pack
    sha256        TEXT NOT NULL,                    -- hash of the canonical bundle
    status        TEXT NOT NULL,                    -- pack status at snapshot time
    done_score    INTEGER NOT NULL DEFAULT 0,
    risk_score    INTEGER NOT NULL DEFAULT 0,
    bundle_json   TEXT NOT NULL,                    -- frozen {pack, artifacts(capped), contract, badges}
    report_md     TEXT NOT NULL DEFAULT '',
    report_html   TEXT NOT NULL DEFAULT '',
    note          TEXT NOT NULL DEFAULT '',
    created_by    TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE INDEX idx_proof_snapshots_pack ON proof_snapshots(proof_pack_id, seq);

-- Media blobs (screenshot/video). Content-addressed by sha256; cascade on the
-- owning artifact's delete. Capped at MEDIA_CAP (25 MiB) by the engine.
CREATE TABLE proof_blobs (
    id           TEXT PRIMARY KEY,
    artifact_id  TEXT NOT NULL REFERENCES proof_artifacts(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL,
    sha256       TEXT NOT NULL,
    mime         TEXT NOT NULL,
    size_bytes   INTEGER NOT NULL,
    data         BLOB NOT NULL,
    created_at   TEXT NOT NULL
);
CREATE INDEX idx_proof_blobs_artifact ON proof_blobs(artifact_id);
