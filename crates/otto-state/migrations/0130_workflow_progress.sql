-- Rebuildable read projections. Recovery JSON remains authoritative.
ALTER TABLE workflow_runs ADD COLUMN progress_json TEXT;
ALTER TABLE workflow_runs ADD COLUMN checkpoint_rev INTEGER NOT NULL DEFAULT 0;
ALTER TABLE workflow_runs ADD COLUMN checkpoint_generation INTEGER NOT NULL DEFAULT 0;
ALTER TABLE workflow_checkpoints ADD COLUMN summary_json TEXT;

-- Generic restore/raw writes also invalidate projections. Ordinary repository
-- writes republish after the authoritative write within the same transaction.
CREATE TRIGGER workflow_progress_insert AFTER INSERT ON workflow_runs BEGIN
    UPDATE workflow_runs SET progress_json=NULL WHERE id=NEW.id;
END;
CREATE TRIGGER workflow_progress_body AFTER UPDATE OF nodes_json ON workflow_runs BEGIN
    UPDATE workflow_runs SET progress_json=NULL, rev=MAX(rev,OLD.rev+1) WHERE id=NEW.id;
END;
CREATE TRIGGER workflow_progress_metadata AFTER UPDATE OF status,error,finished_at,waiting_approval,approval_node_id,approved_by,approval_note,approved_at,workflow_version,proof_pack_id,resume_attempts ON workflow_runs
WHEN NEW.rev = OLD.rev BEGIN
    UPDATE workflow_runs SET rev=rev+1 WHERE id=NEW.id;
END;
CREATE TRIGGER workflow_checkpoint_insert AFTER INSERT ON workflow_checkpoints BEGIN
    UPDATE workflow_checkpoints SET summary_json=NULL WHERE run_id=NEW.run_id AND node_id=NEW.node_id;
    UPDATE workflow_runs SET rev=rev+1,checkpoint_rev=checkpoint_rev+1 WHERE id=NEW.run_id;
END;
CREATE TRIGGER workflow_checkpoint_body AFTER UPDATE OF checkpoint_json ON workflow_checkpoints BEGIN
    UPDATE workflow_checkpoints SET summary_json=NULL WHERE run_id=NEW.run_id AND node_id=NEW.node_id;
    UPDATE workflow_runs SET rev=rev+1,checkpoint_rev=checkpoint_rev+1 WHERE id=NEW.run_id;
END;
CREATE TRIGGER workflow_checkpoint_delete AFTER DELETE ON workflow_checkpoints BEGIN
    UPDATE workflow_runs SET rev=rev+1,checkpoint_rev=checkpoint_rev+1,checkpoint_generation=checkpoint_generation+1 WHERE id=OLD.run_id;
END;
