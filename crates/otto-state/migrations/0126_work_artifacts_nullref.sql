-- work_artifacts: make a NULL `ref` dedupe like every other ref does.
--
-- 0083 gave the table UNIQUE(work_item_id, kind, ref), but SQLite treats NULLs
-- as DISTINCT inside a UNIQUE constraint, so the `INSERT OR IGNORE` in
-- WorkGraphRepo::add_artifact never ignored anything for the projector's
-- NULL-ref rows (the review verdict report + the PR link). Every 5-minute
-- workgraph reconcile therefore appended a FRESH copy of each: one work item
-- reached ~99k artifact rows, and the read-back that follows the insert had to
-- walk the whole (item, kind) bucket through a temp B-tree — 1.2-2.5 s per
-- call, logged by sqlx as `slow statement`, holding one of the 8 pool
-- connections and stalling everything else that wanted SQLite (the terminal
-- WebSocket awaits a `sessions` read on every keystroke, so it surfaced as
-- typing lag every 5 minutes).
--
-- Collapsing duplicates DELETES rows, which is safe here and only here:
-- `work_artifacts` holds DERIVED projection rows written by
-- `workgraph_projector` from the authoritative repos (reviews, sessions, goal
-- loops, …). They are re-created by the next reconcile/backfill sweep, they
-- carry no user-authored content, and the surviving row per key is the OLDEST
-- one — i.e. the original, with its original `created_at`. Nothing else in the
-- DB is touched.

DELETE FROM work_artifacts
 WHERE "ref" IS NULL
   AND id NOT IN (
       SELECT id FROM (
           SELECT id,
                  ROW_NUMBER() OVER (
                      PARTITION BY work_item_id, kind
                      ORDER BY created_at ASC, rowid ASC
                  ) AS rn
             FROM work_artifacts
            WHERE "ref" IS NULL
       )
        WHERE rn = 1
   );

-- The missing half of the uniqueness rule: one NULL-ref artifact per
-- (work_item_id, kind). Non-NULL refs keep the 0083 UNIQUE. `add_artifact`
-- inserts with ON CONFLICT DO NOTHING, so both keys are honoured by one insert.
CREATE UNIQUE INDEX ux_work_artifacts_item_kind_nullref
    ON work_artifacts(work_item_id, kind) WHERE "ref" IS NULL;
