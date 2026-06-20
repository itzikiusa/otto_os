-- Per-node output cache for workflow re-runs. When a node's params and
-- upstream input are unchanged from a prior successful run, the engine
-- can skip re-executing it and surface the stored output as "cached".
--
-- Key: (workflow_id, node_id, params_hash, input_hash)
-- Value: the JSON output produced by that node.
CREATE TABLE workflow_node_cache (
    id           TEXT PRIMARY KEY,
    workflow_id  TEXT NOT NULL REFERENCES workflows(id) ON DELETE CASCADE,
    node_id      TEXT NOT NULL,
    -- SHA-256 (hex) of the canonical JSON of the node's `params` field.
    params_hash  TEXT NOT NULL,
    -- SHA-256 (hex) of the canonical JSON of the node's assembled input.
    input_hash   TEXT NOT NULL,
    -- The output value produced by the node (arbitrary JSON).
    output_json  TEXT NOT NULL,
    created_at   TEXT NOT NULL
);
-- Fast lookup by the natural key: workflow + node + both hashes.
CREATE UNIQUE INDEX idx_wf_node_cache ON workflow_node_cache(workflow_id, node_id, params_hash, input_hash);
