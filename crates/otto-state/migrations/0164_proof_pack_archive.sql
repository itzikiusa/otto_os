-- Perf wave (R3): opt-in archiving of stale, evidence-less session proof packs.
-- Additive only. Nothing is deleted: `POST /workspaces/{id}/proof-packs/archive-sessions`
-- (an explicit, dry-run-by-default admin action) only stamps `archived_at`,
-- which hides the pack from the summary roll-up and the default pack list.

ALTER TABLE proof_packs ADD COLUMN archived_at TEXT;

-- Keyset walk of the live (un-archived) packs: list + summary filter on
-- `archived_at IS NULL` and order by (updated_at, id).
CREATE INDEX IF NOT EXISTS idx_proof_packs_ws_archived_updated
    ON proof_packs(workspace_id, archived_at, updated_at, id);

-- A pack un-archives itself the moment it changes or gains evidence, however
-- the change is made (recompute, waive, meta edit, repo link, new artifact).
-- The inner UPDATE only touches `archived_at`, which is not in the OF list,
-- so the trigger never re-fires on itself.
CREATE TRIGGER IF NOT EXISTS trg_proof_packs_unarchive_on_update
AFTER UPDATE OF status, risk_score, done_score, title, summary, updated_at,
                parent_pack_id, repo_id, pr_number, waived_by, waived_reason
ON proof_packs
WHEN NEW.archived_at IS NOT NULL
BEGIN
    UPDATE proof_packs SET archived_at = NULL WHERE id = NEW.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_proof_packs_unarchive_on_artifact
AFTER INSERT ON proof_artifacts
BEGIN
    UPDATE proof_packs SET archived_at = NULL
     WHERE id = NEW.proof_pack_id AND archived_at IS NOT NULL;
END;
