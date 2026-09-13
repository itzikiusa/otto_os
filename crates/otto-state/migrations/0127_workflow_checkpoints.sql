-- Durable loop attempts. A running external operation has an unknown outcome
-- after interruption; successful steps are adopted without repeating it.
CREATE TABLE workflow_checkpoints (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    checkpoint_json TEXT NOT NULL,
    PRIMARY KEY (run_id, node_id)
);
