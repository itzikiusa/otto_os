-- Memory lifecycle governance: state machine, soft-delete with undo, provenance
-- links for merge/split, and a governed-import asset tracker. Added on top of
-- 0038_memory.sql (memories table) and 0039_memory_sharing.sql (visibility).

-- Lifecycle state column. Allowed values:
--   suggested  — auto-ingested, awaiting human review
--   accepted   — active and approved (default for manually created memories)
--   stale      — no longer current; soft-deprecated
--   contradicted — overridden by a newer memory (set automatically on merge/split)
ALTER TABLE memories ADD COLUMN state TEXT NOT NULL DEFAULT 'accepted';

-- Provenance: JSON-encoded record of how a memory came to be (merge/split/import).
-- Format: { "op": "merge|split|import", "source_ids": [...], "kind": "agents-md|..." }
ALTER TABLE memories ADD COLUMN provenance_json TEXT;

-- superseded_by: already existed in 0038, no-op here — kept for documentation.
-- (It was: ALTER TABLE memories ADD COLUMN superseded_by TEXT;)

-- Soft-delete timestamp (Unix epoch seconds). NULL = not forgotten; > 0 = forgotten.
-- `active` (0038) covers the immediate filter; forgotten_at is the audit trail.
ALTER TABLE memories ADD COLUMN forgotten_at INTEGER;

-- Opaque random token used to undo a forget within the retention window.
-- Cleared once the forget is made permanent or after undo restores the row.
ALTER TABLE memories ADD COLUMN undo_token TEXT;

-- Index: filter by lifecycle state (e.g. list only 'suggested' for review).
CREATE INDEX idx_memories_state ON memories(workspace_id, state, active);

-- Index: undo-forget token lookup (point lookup, not a range scan).
CREATE UNIQUE INDEX idx_memories_undo_token ON memories(undo_token) WHERE undo_token IS NOT NULL;

-- Governed-asset import tracker: records which AGENTS.md / CLAUDE.md /
-- .cursorrules files have been imported into the memory layer and makes the
-- import revertible. One row per import batch; links to the individual memory
-- rows via `memory_ids_json` (a JSON array of memory ids).
CREATE TABLE governed_imports (
  id              TEXT PRIMARY KEY,
  workspace_id    TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
  kind            TEXT NOT NULL,    -- agents-md | claude-md | cursorrules | custom
  label           TEXT NOT NULL,    -- human-readable file label (path or title)
  memory_ids_json TEXT NOT NULL DEFAULT '[]',   -- JSON array of created memory ids
  imported_by     TEXT NOT NULL,
  imported_at     TEXT NOT NULL,
  reverted_at     TEXT             -- NULL until the import is reverted
);
CREATE INDEX idx_governed_imports_ws ON governed_imports(workspace_id, reverted_at);
