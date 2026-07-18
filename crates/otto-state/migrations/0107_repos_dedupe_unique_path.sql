-- Repos accumulated duplicate rows for the same filesystem path (bulk imports
-- and agent runs re-registered already-known repos into other workspaces),
-- so the same checkout showed up as several independent "repos" with
-- diverging state. Merge them — oldest row per path survives, every child
-- reference is re-pointed — and enforce uniqueness so it cannot recur.
-- (create_repo in otto-state normalizes paths and upserts against this index.)

CREATE TEMP TABLE repo_dedupe_map AS
SELECT l.id AS loser,
       (SELECT s.id FROM repos s
         WHERE s.path = l.path
         ORDER BY s.created_at, s.id
         LIMIT 1) AS survivor
FROM repos l;

-- Re-point every table that stores a repo id. Plain-TEXT references included:
-- only review_findings has a real FK, the rest are by-convention columns.
-- (code_text_chunks also holds repo_id in some long-lived DBs, but it is an
-- orphaned dev-run table no code reads and no migration creates — skipped.)
UPDATE pr_reviews       SET repo_id = (SELECT survivor FROM repo_dedupe_map WHERE loser = repo_id)
 WHERE repo_id IN (SELECT loser FROM repo_dedupe_map WHERE loser <> survivor);
UPDATE proof_packs      SET repo_id = (SELECT survivor FROM repo_dedupe_map WHERE loser = repo_id)
 WHERE repo_id IN (SELECT loser FROM repo_dedupe_map WHERE loser <> survivor);
UPDATE review_findings  SET repo_id = (SELECT survivor FROM repo_dedupe_map WHERE loser = repo_id)
 WHERE repo_id IN (SELECT loser FROM repo_dedupe_map WHERE loser <> survivor);
UPDATE work_items       SET repo_id = (SELECT survivor FROM repo_dedupe_map WHERE loser = repo_id)
 WHERE repo_id IN (SELECT loser FROM repo_dedupe_map WHERE loser <> survivor);
UPDATE otto_runs        SET repo_id = (SELECT survivor FROM repo_dedupe_map WHERE loser = repo_id)
 WHERE repo_id IN (SELECT loser FROM repo_dedupe_map WHERE loser <> survivor);

DELETE FROM repos
 WHERE id IN (SELECT loser FROM repo_dedupe_map WHERE loser <> survivor);

DROP TABLE repo_dedupe_map;

CREATE UNIQUE INDEX idx_repos_path ON repos(path);
