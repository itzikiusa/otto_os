-- Perf wave: indexes behind scheduled Design Hall retention / blob GC and
-- keyset paging, plus Proof pack paging + the content-addressed media store.
-- Additive only.

-- ── Design Hall ──────────────────────────────────────────────────────────
-- Blob GC asks "is this sha still a thumbnail / a pinned version?" per
-- candidate; without these it is a full scan per blob.
CREATE INDEX IF NOT EXISTS idx_design_artifacts_thumb ON design_artifacts(thumb_blob)
    WHERE thumb_blob IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_design_links_pinned ON design_links(pinned_version_id)
    WHERE pinned_version_id IS NOT NULL;
-- Keyset cursor (updated_at, id) across workspaces and per studio.
CREATE INDEX IF NOT EXISTS idx_design_artifacts_updated ON design_artifacts(updated_at, id);
CREATE INDEX IF NOT EXISTS idx_design_artifacts_studio_updated ON design_artifacts(studio, updated_at);
-- Render-adjacency BFS: one query per level over (src, rel).
CREATE INDEX IF NOT EXISTS idx_design_links_src_rel ON design_links(src_artifact_id, rel);

-- ── Proof packs ──────────────────────────────────────────────────────────
-- (proof section appended below)
-- `GET /workspaces/{id}/proof-packs` orders by updated_at and pages on the
-- (updated_at, id) keyset; only (workspace_id, status) was indexed.
CREATE INDEX IF NOT EXISTS idx_proof_packs_ws_updated
    ON proof_packs(workspace_id, updated_at, id);
-- Media moves out of the state DB into a content-addressed file store under
-- the daemon data dir (`proof-media/<sha[..2]>/<sha>`). `stored = 1` means the
-- bytes live in the file and `data` is an empty placeholder (the column is NOT
-- NULL); `stored = 0` rows still carry their bytes and are moved by the
-- background migrator only after the file is fsynced and read back verified.
ALTER TABLE proof_blobs ADD COLUMN stored INTEGER NOT NULL DEFAULT 0;
-- Dedupe + GC ask "is this sha still referenced?".
CREATE INDEX IF NOT EXISTS idx_proof_blobs_sha ON proof_blobs(sha256, stored);
