# Iteration 2 correctness recheck — data tools

**Verdict:** Block
**Counts:** blocker 1 · major 2 · minor 0 · question 0
**Scope:** Independent source recheck of the uncommitted role-2 repairs against baseline `a16f4c71`, the original correctness report, and `iteration-1-implementation-2.md`. Reviewed the repaired paths plus immediately adjacent failure cases. No source edits, builds, tests, servers, or live writes were performed; only this report was written.

**Evidence status:** Findings are confirmed by concrete hand traces. Parent-reported red/green checks are useful coordination evidence, but are not tests run by this reviewer. Full verification gates remain pending.

## Remaining findings

### C2-R2-01 [blocker] PostgreSQL unquoted identifiers can still retarget an edit to another table — `ui/src/modules/database/edit-sql.ts:117`

**Intended:** The new conservative parser must establish the actual source table of each editable row. Quoted and unquoted PostgreSQL identifiers have different case semantics.

**What:** `unquote()` removes quote characters but preserves the spelling of unquoted identifiers. That name is used for exact catalog lookup, and subsequently emitted as a quoted identifier. PostgreSQL folds unquoted identifiers to lowercase; the generated mutation can therefore target a different table from the SELECT.

**Evidence:** Confirmed, hand-traced. Create two ordinary PostgreSQL tables in schema `app`: `users` and `"Users"`, each with `id INT PRIMARY KEY, name TEXT`, each containing `id=1`, but with different names. Run `SELECT id, name FROM app.Users WHERE id = 1`.

1. PostgreSQL resolves the unquoted `Users` in this query as `users`; the displayed row comes from `app.users`.
2. The repaired parser at `edit-sql.ts:115` accepts it. Lines 117 and 133 return `{db:'app', table:'Users'}` without folding the unquoted name.
3. `EditFlow.resolveTarget` constructs `db:app/table:Users` and fetches its metadata (`ui/src/modules/database/EditFlow.svelte.ts:232` onward). The PostgreSQL driver's catalog queries use exact `n.nspname = $1 AND c.relname = $2` comparisons (`crates/otto-dbviewer/src/drivers/postgres.rs:407`, `:419`) with that spelling. They find the separate `app."Users"` table and its matching `id` key.
4. Editing the displayed name produces `UPDATE "app"."Users" SET "name" = 'changed' WHERE "id" = 1` through `tableRef` and `buildUpdate`. This changes the other table, while the source row in `app.users` stays unchanged. The same error affects generated DELETE and schema-name folding.

**Fix:** Normalize parsed identifiers according to the engine before catalog lookup or generated SQL: for PostgreSQL, lowercase unquoted identifiers and preserve double-quoted identifiers exactly. Preserve quote provenance during parsing. Reject syntax whose engine semantics are not established instead of translating it by merely removing quotes.

**Regression:** Add parser cases for PostgreSQL `app.Users`, `APP.users`, `app."Users"`, and `"APP".users`. In an isolated PostgreSQL fixture with both `users` and `"Users"`, assert that an edit from the unquoted query modifies only lowercase `users`; the explicitly quoted query must target `"Users"`. This is a remaining provenance hole adjacent to C2-01; the original aliased-key trigger is fixed.

### C2-R2-02 [major] New batch limit injection breaks valid explicit-limit statements — `crates/otto-dbviewer/src/drivers/postgres.rs:1926`

**Intended:** P2-02 limits only eligible SELECTs and preserves SQL/session semantics for explicit limits and other non-rewritable statements. The implementation handoff and injector's own contract both state this exception.

**What:** The new batch calls reuse an injector that recognizes an existing LIMIT only when followed by spaces and a digit. Valid PostgreSQL `LIMIT ALL`, parenthesized limit expressions, and a numeric LIMIT separated by a newline are rewritten with an additional LIMIT clause.

**Evidence:** Confirmed, hand-traced. Submit the normal batch `SELECT id FROM app.users LIMIT ALL; SELECT 42 AS marker;` with `max_rows=2`.

1. `run_batch` recognizes the first statement as a read and now passes it to `inject_row_limit(stmt, 3, None)` at `postgres.rs:1926`. Before this repair, this batch path passed the original statement unchanged.
2. `types.rs:1238` calls `has_word_then_digit(..., "limit")`. The character after `LIMIT ` is `A`, so the function returns false. None of the `SKIP` clauses at lines 1242–1250 matches.
3. `types.rs:1261` returns `SELECT id FROM app.users LIMIT ALL LIMIT 3`, which is invalid SQL. `run_batch` appends an errored entry and breaks at `postgres.rs:1953` onward, so the marker statement never runs.
4. `LIMIT (2)` follows the same failing path. `LIMIT\n2` also fails recognition because `has_word_then_digit` skips only ASCII spaces, then receives a second LIMIT. The newline case also affects MySQL's new batch rewrite. The governed batch paths use the same injector.

**Fix:** Detect existing LIMIT clauses using SQL tokens outside comments/quoted text, accepting all supported limit forms and whitespace; keep the original SQL when the injector cannot prove a rewrite is valid. At minimum, refuse to inject after any actual LIMIT token, and cover PostgreSQL FETCH clauses/locking variants under the same conservative eligibility policy. Do not change the shared session to avoid this failure.

**Regression:** Assert unchanged SQL and `limited=false` for `LIMIT ALL`, `LIMIT (2)`, and `LIMIT\n2`. Exercise a two-statement PostgreSQL batch for each form and verify the second statement executes on the same backend session. Exercise MySQL numeric LIMIT separated by a newline. Keep the existing plain unconstrained SELECT case proving the server cap is still applied.

### C2-R2-03 [major] Workspace switching bypasses the automation leave guard and loses edits — `ui/src/modules/api/ApiPage.svelte:132`

**Intended:** UX2-02 protects the component-local automation working copy before navigation destroys its editor, using Save/Discard/Cancel.

**What:** The new guards cover router navigation and local view transitions, but workspace selection directly changes `ws.currentId`. The API page then assigns the request view unconditionally, unmounting the dirty editor without asking.

**Evidence:** Confirmed, hand-traced. Open an automation in workspace A and change an assertion; `patchStep` sets the local `dirty=true`. Click workspace B in the navigator.

1. `ui/src/shell/Navigator.svelte:956` calls `ws.select(w.id)` directly. Other workspace entry points do likewise.
2. `ui/src/lib/stores/workspace.svelte.ts:539`–547 changes `currentId` before awaiting any load, without running the router's leave guards.
3. The API page's workspace effect at lines 129–135 executes `view = {kind:'request'}` and loads workspace B. It does not call `changeView` or `automationEditor.approveLeave`.
4. The keyed AutomationEditor is removed. Its steps exist only in component state; no save or retained draft occurred. Returning to A reloads the saved definition, so the changed assertion is gone. The new `router.guard` registration never participates because the route need not change.

**Fix:** Make user-initiated workspace changes participate in the active pane's leave decision before mutating workspace identity, or durably retain automation drafts per workspace. A page-level asynchronous prompt after `currentId` changes is too late, and a Save at that point can target the wrong workspace API base. Preserve generation checks so a canceled/superseded transition cannot later switch workspaces.

**Visible entry points / integration:** `Navigator.svelte:328` (filter Enter), `:340` (workspace chip menu), `:956` (workspace row), `:958` (context-menu switch), `:967` (workspace context action), `ui/src/shell/App.svelte:730` (registered command-palette switch), and `ui/src/lib/components/FloatingBar.svelte:125` (space selection) call `ws.select` directly. App also has event-driven calls at lines 878 and 989; distinguish these from startup restoration when deciding guard policy. The existing router exposes `guard(fn)` at `ui/src/lib/router.svelte.ts:303`; execution is currently the private `mayLeave(hash)` at line 312, which already rejects superseded decisions via `guardSeq`. Reuse/expose an appropriate pre-context-change decision rather than making every caller navigate artificially. The workspace-context action must also honor a canceled workspace switch before routing to settings.

**Regression:** In the rendered application, edit an automation and switch workspace via navigator and command palette. Cancel must preserve workspace A and its exact draft; failed Save must also stay; successful Save must persist to A before B becomes current; Discard may proceed. Returning after a saved transition must show the saved assertion.

## Repairs that hold under this recheck

| Repair | Source cases traced and result |
|---|---|
| C2-01 original aliased/computed key | Projection grammar rejects aliases, expressions, duplicate names, extra FROM sources, and MySQL double-quoted projection strings. The original wrong-row fixture is rejected. Identifier folding remains open above. |
| C2-02 ClickHouse row mutations | `resolveTarget` declines ClickHouse before adopting key metadata; update/delete/replace builders independently return null. A matching key on one visible page cannot enable row mutation. Copy-as-INSERT remains separate. |
| C2-03 staging restoration | Successful auto-stashed checkout and shared rollback now use `stash pop --index`. Index restoration errors retain the stash; no silent fallback to non-index pop was added. The failed-checkout and same-HEAD switch traces preserve the intended staging partition when restoration succeeds. |
| C2-04 script variable races | Explicit set/unset writes flow through `ScriptRun.writes`; both pre/post publication merge against current workspace variables. A slow read-only script has an empty write set, so it cannot undo another tab's write. Disjoint keys, manual edits, explicit same-value writes, and unset preserve the stated per-key completion policy. Erroring scripts are not published by these paths. |
| P2-01 Mongo final shaping | Exact serialized column/cell bytes are charged before retaining a complete row; padding nulls and late columns count; the 1,000,000-cell cap applies to retained rectangular rows. Partial rows are dropped, byte truncation propagates through `collect_docs`, and oversized metadata returns cleared columns. Traced empty docs, empty documents, late widening, spill, and failed-cell-budget branches. No new correctness defect established. |
| P2-02 session ownership | Normal batches retain one acquired connection; governed batches retain one transaction. Changes do not split statements across sessions or add batch offsets. Injector eligibility remains open above. |
| P2-03 lazy schema bodies | List generation and abort signals guard replaced subjects/clusters. Per-side controllers invalidate an older A/B selection; independent errors and retry remain usable. Cache keys include the full cluster/subject base plus numeric version, with byte/entry eviction. No remaining defect established in the changed list/detail paths. |
| UX2-01/02 local navigation | Dirty scratch close asks before removal and resolves the stable tab ID afterward. Automation Save/Discard/Cancel coalesces concurrent prompts; view transitions honor the result; edits arriving during Save retain dirty status. Workspace navigation remains open above. |
| UX2-03 broker tail | Failed increments preserve the buffer/cursors, clear the new-message count, expose error/retry, and successful empty increments refresh freshness. Different-topic replies are rejected by captured URL ownership. |

## Limits and next verification

This was a finite focused source pass. I did not execute the supplied regressions, render Svelte components, run SQL/Mongo/Kafka services, or reproduce Git staging commands. The implementation report's tests and parent-reported greens are not substitutes for the new cases above. External Git writers, forced window termination, every SQL dialect construct, broker compatibility-check requests, and all backend transport paths were not re-audited. No design or performance verdict is implied.
