-- Durable per-agent retry (review-engine reliability design, 2026-07-04):
-- the fully-composed prompt for each review agent, persisted at dispatch time
-- so POST /reviews/{id}/agents/{index}/retry survives reboots, periodic
-- temp-dir sweeps, and daemon redeploys (the $TMPDIR prompt file remains the
-- legacy fallback for reviews that predate this migration). Rows are deleted
-- by the cancel cleanup — the same lifecycle as the temp files they replace.
CREATE TABLE review_agent_prompts (
  review_id   TEXT NOT NULL,
  agent_index INTEGER NOT NULL,
  prompt      TEXT NOT NULL,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  PRIMARY KEY (review_id, agent_index)
);

-- The diff is NOT embedded in the prompt: agents read it from a temp file whose
-- absolute path is baked into the prompt text. A durable retry must therefore
-- be able to re-materialize that temp file after a sweep, so the review's
-- unified diff is persisted once per run alongside the prompts.
CREATE TABLE review_diffs (
  review_id  TEXT PRIMARY KEY,
  diff       TEXT NOT NULL,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
