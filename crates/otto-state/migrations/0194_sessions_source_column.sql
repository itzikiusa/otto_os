-- The sidebar's "shown sessions" filter (`SessionsRepo::list_filtered` with
-- `foreground`, `visible_ids`, the activity summary's shown-only trail) used
-- to evaluate `json_type`/`json_extract(meta_json, '$.source')` on EVERY row
-- of a workspace — ~2 k background review agents parsed per list to return
-- ~60 rows (32 ms cold on the field DB).
--
-- `source` is `meta.source` when it is a JSON string, else NULL — exactly the
-- rule `Session::is_foreground_agent` applies. VIRTUAL (no table rewrite, no
-- stored bytes); the `json_valid` guard keeps a malformed `meta_json` from
-- failing the index build (CASE short-circuits). Note: `PRAGMA table_info`
-- does not list generated columns while `SELECT *` returns them — readers
-- that copy rows generically must name their columns (see state_archive).
ALTER TABLE sessions ADD COLUMN source TEXT GENERATED ALWAYS AS (
    CASE WHEN json_valid(meta_json) AND json_type(meta_json, '$.source') = 'text'
         THEN json_extract(meta_json, '$.source') END
) VIRTUAL;

-- Same leading columns and order as 0158's list index (it serves the list
-- sort and the archived pager unchanged), plus `kind` and `source` so the
-- shown-row predicate is decided from the index entry — the row (and its
-- meta JSON) is only read for rows that match. The 0158 index is a strict
-- prefix of this one; left in place the planner keeps picking it (it can't
-- tell them apart without ANALYZE) and re-parses the JSON, so it goes.
CREATE INDEX IF NOT EXISTS idx_sessions_ws_arch_created_src
    ON sessions(workspace_id, archived, created_at, id, kind, source);
DROP INDEX IF EXISTS idx_sessions_ws_archived_created;
