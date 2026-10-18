# Iteration 04 — editor and docs, independent read

Read after Mongo 1.3 live, the focused suite, and distinct live were reported green. No Cargo, no UI re-run, no doc edits. Root is updating docs. 9.8 is not certified here.

| Lens | Score | This pass |
|---|---:|---|
| L21 query routing | 9.5 | Unchanged. UI `DELIMITER` still sends the whole buffer; the server splitter owns the delimiter. Not re-run. |
| L22 dialog | 9.7 | `Modal.svelte` height is still optional and `max-height` is still the backdrop. Browser cases not repeated. |
| L23 errors / docs | 9.3 | Grid tooltips still say raise the row limit or narrow the query. The feature doc already names the buffered fallback. |
| L24 release | 9.1 | UI unit green was given earlier and not repeated. Engine install and the access Cargo slot are still open. |

## Docs checked, not edited

`docs/features/database-explorer.md` export section says the daemon streams MySQL, MongoDB cursors, and ClickHouse HTTP `FORMAT` bytes. The next bullet says Redis, a ClickHouse JSON array, and the native transport buffer the result. The earlier note that this page only claimed unbounded streaming is out of date.

`ui/src/modules/help/sections/database.md` says a bare read receives the row cap, and that Stop cancels MongoDB when the account has `inprog` and `killop`. That matches the driver: cancel looks up `comment: "otto:<server id>"` and `killOp`s it. The help text does not mention the client `query_id`. It does not need to. The id is not the cancel tag (`mongo_comment_tag` in `mongodb.rs`).

`ResultsGrid.svelte` row-cap title is still "Raise the row limit or narrow the query to fetch more." The byte badge says to narrow the query. Neither promises `LIMIT` or a streaming export.

## Left for root

Doc consolidation, the access Cargo run, and the one PR plus required CI, rebuild, and install. This pass found no new editor defect and did not change product code outside the access error text already queued for that Cargo run.
