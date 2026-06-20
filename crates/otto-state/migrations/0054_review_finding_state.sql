-- A1 Verified review loop: extend review_findings with lifecycle state columns,
-- fixing the committed vs. run-tracked distinction. The original 0049 migration
-- used `status` as the lifecycle column; we add a dedicated `state` column
-- (open|fixing|resolved|regressed|declined) plus fix-attribution fields so the
-- UI can key findings stably by fingerprint and track cross-run transitions.
--
-- We also add a `review_id` column (the *current* run the finding last appeared
-- in) distinct from `first_seen_review_id`/`last_seen_review_id` from 0049.

ALTER TABLE review_findings ADD COLUMN state TEXT NOT NULL DEFAULT 'open';
ALTER TABLE review_findings ADD COLUMN fix_session_id TEXT;
ALTER TABLE review_findings ADD COLUMN fix_commit TEXT;
ALTER TABLE review_findings ADD COLUMN first_seen_run TEXT;
ALTER TABLE review_findings ADD COLUMN last_seen_run TEXT;
ALTER TABLE review_findings ADD COLUMN category TEXT;
-- `review_id`: the most-recent review run that produced / re-confirmed this finding.
ALTER TABLE review_findings ADD COLUMN review_id TEXT REFERENCES reviews(id) ON DELETE CASCADE;

-- Composite index used by the lifecycle-upsert path and findings list queries.
CREATE INDEX IF NOT EXISTS idx_review_findings_review_fp
    ON review_findings (review_id, fingerprint);
