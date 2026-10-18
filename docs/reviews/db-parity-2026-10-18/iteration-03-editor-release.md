# Iteration 03 — L21–L24 independent read

Read-only pass over shared types, the statement splitter, the query editor, the modal, and the truncation copy. No driver edits, no Cargo, no UI test run. The current UI gate given for this pass is `npm run check` at 0 errors and 0 warnings, and the UI unit suite green. That gate was not repeated here. 9.8 is not newly certified.

| Lens | This pass | Notes |
|---|---:|---|
| L21 query routing | 9.5 | Shared Rust splitter and the editor's "send the whole buffer" path agree. Crate tests not re-run. |
| L22 dialog / accessibility | 9.7 | Sheet height is still optional and capped to the backdrop. Prior browser cases were not repeated, so this pass does not re-certify 9.8. |
| L23 contracts / errors | 9.2 | Row-cap tooltip no longer promises a SQL `LIMIT`. Workflow host-directory error is the access follow-up in iteration-03-access. |
| L24 regressions / release | 9.0 | Shared session-state detector covers comment/whitespace before `GET_LOCK` / `SET_CONFIG`. Combined engine gate and install are still open. UI unit green does not cover that. |

## L21

`crates/otto-dbviewer/src/split.rs` recognizes a MySQL `DELIMITER` only at the start of a statement, including a quoted token that may contain spaces (`DELIMITER 'x y'`). The directive is not returned as SQL.

The editor does not re-implement that split. `hasMysqlDelimiter` in `ui/src/modules/database/sql-util.ts` treats a real `DELIMITER` line (mask says it is code, not a string or comment) as one buffer. `QueryEditor.run` then sends the whole script when there is no selection, and the server splits it. A quoted delimiter with spaces still matches the UI detector (`[^\r\n]+`), so Run does not slice the routine body locally. Selection and "Run all" are unchanged: selection wins; Run all sends `tab.statement`.

No new routing defect in that pair. The splitter's own 26 tests and the editor/dialect tests were not executed in this pass.

## L22

`Modal.svelte` applies `height` only when the prop is set (`style:height`), and `.sheet` keeps `max-height: calc(100% - 64px)` against the backdrop (`inset: 0`), with `min-height: 0` so the body can scroll. A fixed height cannot grow past the window. This matches the earlier fix. The four light/dark desktop/phone cases were not opened again.

## L23

`ResultsGrid.svelte` byte-budget badge says to narrow the query. The row-cap badge says to raise the row limit or narrow the query. Neither tooltip tells the user to add `LIMIT` or to use a streaming export.

`ui/src/modules/help/sections/database.md` still says a bare read receives the row cap. That is the product behavior, and it is separate from the badge. Feature-doc consolidation stays with root. One mismatch to keep on that list: `docs/features/database-explorer.md` still describes daemon streaming in places where the ClickHouse HTTP JSON export remains the buffered fallback. That is a doc residual, not a UI regression, and this pass did not edit it.

The workflow error finding is fixed in the access patch (iteration-03-access), not in the editor.

## L24

`sql_leaves_session_state` in `crates/otto-dbviewer/src/types.rs` tokenizes through comments and whitespace, then matches `GET_LOCK`, `ADVISORY_LOCK`, and `SET_CONFIG`. A hit inside a string literal can retire a pooled connection; the comment records that as the safe direction. Call sites in the MySQL, Postgres, and ClickHouse drivers were not modified and were not re-run.

UI check and UI unit green, as given, cover the editor and modal types. They do not replace the live engine suite or the install gate. No score here is 9.8.
