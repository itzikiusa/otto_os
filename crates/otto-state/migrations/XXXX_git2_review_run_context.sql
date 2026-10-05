-- Where a review run looked at the code (S2-05 / S2-06). Every local,
-- Run-with-Otto and workflow branch review shares the `pr_number = 0`
-- sentinel, so without the source branch one branch's review resolved open
-- findings another branch raised in the same file; and without the checkout
-- path + head a Retry ran the reviewer in the user's main checkout (usually on
-- another branch) while the prompt still promised the source branch.
-- One row per review; written when the run starts, read by retries and the
-- finding-resolution scope. Additive: absent ⇒ legacy behaviour.
CREATE TABLE review_run_context (
  review_id     TEXT PRIMARY KEY,
  source_branch TEXT,
  cwd           TEXT,
  head_sha      TEXT,
  created_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
