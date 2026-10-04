# Correctness review — partition 2

**Verdict:** Block
**Counts:** blocker 2 · major 2 · minor 0 · nit 0 · question 0
**Baseline:** `a16f4c71d5b158359fc3507c9f883c500c1f88a3`
**Intended behavior reviewed against:** Database Explorer's uniquely addressable inline edits (`docs/features/database-explorer.md:1458`), the edit flow's exact-row invariant, Git auto-stash's restoration contract, and independent API request tabs with workspace-scoped script variables.
**Scope traced:** Existing application code, not just a diff. SQL edit detection and UPDATE/DELETE construction; ClickHouse key metadata; Git checkout/stash restoration and stage/discard operations; API request snapshotting, save completion, and script variable publication; broker group reset selection/confirmation/resolution; SFTP navigation and transfer initiation.

All findings below are confirmed by hand-tracing concrete inputs through the cited code. No builds, servers, database writes, Git mutations, or executable reproductions were run. The only authored file is this report.

## Findings

### C2-01 [blocker] Projection aliases can make an inline edit update a different row — `ui/src/modules/database/edit-sql.ts:108`

**What:** Editability accepts computed/aliased primary-key projections as real primary keys. Generated UPDATE/DELETE statements use that displayed value to identify a base-table row.

**Intended:** An editable result must retain the actual identity of each base-table row; edits must target the row that produced the result. `EditFlow.svelte.ts:231` explicitly states the exact-row requirement.

**Why it matters:** Editing a displayed row can silently alter another existing row. The confirmation presents plausible SQL using the displayed key, so it does not establish the missing provenance.

**Evidence:** Confirmed, hand-traced. In MySQL, let `app.users` contain `(id=1, other_id=2, name='Alice')` and `(id=2, other_id=3, name='Bob')`. Run `SELECT other_id AS id, name FROM app.users WHERE id = 1`. The displayed result is `(id=2, name='Alice')`.

1. `parseSimpleSelect` at lines 113–126 rejects batches, joins, and a few aggregates, but accepts this projection and returns `{db:'app', table:'users'}`.
2. `EditFlow.resolveTarget` at `ui/src/modules/database/EditFlow.svelte.ts:237` only checks that some result column is named `id`. It adopts the catalog PK at line 245; it does not verify that the column came from `users.id`.
3. Changing the displayed name to `Alicia` reaches `sqlAdapter.buildUpdate` at `edit-sql.ts:155`. `whereByPk` at line 88 finds the aliased result column and produces `id = 2`.
4. Actual SQL is `UPDATE app.users SET name = 'Alicia' WHERE id = 2` (identifiers quoted in the real builder): Bob's row is changed, while Alice's row remains unchanged. `buildDelete` has the same identity error. `EditFlow.runReview` at line 703 sends the generated statement through the normal write path.

**Fix:** Require proven projection provenance before enabling edits. A conservative initial implementation should accept only a fully parsed single-table projection of `*` or direct, unaliased base columns, with no duplicate result names; reject expressions/aliases until an explicit result-to-base-column mapping exists. Validate the complete FROM clause too, rather than only its first table token. Use the mapping for both key predicates and SET columns.

**Regression:** Seed the two rows above. The aliased query must be read-only, and attempting a UI edit/delete must issue no mutation. Also cover `id + 1 AS id`, a non-key alias onto another real column, duplicate column names, and a normal `SELECT id, name FROM app.users` positive case.

### C2-02 [blocker] ClickHouse sorting keys are treated as unique row identities — `ui/src/modules/database/EditFlow.svelte.ts:233`

**What:** The shared edit flow considers a ClickHouse primary key sufficient to identify one row, although MergeTree primary keys do not enforce uniqueness. Selecting one displayed row can update or delete all rows sharing that key.

**Intended:** Inline edit/delete must act on the selected row, or be unavailable when a row cannot be uniquely addressed. This is the documented editability limitation and the explicit rationale at `EditFlow.svelte.ts:231`.

**Why it matters:** A normal analytical table can contain many events per key. A single-row gesture becomes a broad mutation without identifying those additional rows in its review.

**Evidence:** Confirmed, hand-traced. Consider `app.events (account_id UInt32, event_id UInt32, note String) ENGINE=MergeTree ORDER BY account_id`, containing `(7,101,'a')` and `(7,102,'b')`. Run `SELECT * FROM app.events`.

1. `crates/otto-dbviewer/src/drivers/clickhouse.rs:2206` reads `is_in_primary_key`; line 2222 adds `account_id`, and line 2271 publishes it as `ObjectDetail.primary_key`.
2. `EditFlow.resolveTarget` at lines 233–245 accepts this nonempty key and its presence in the result. There is no uniqueness distinction for ClickHouse.
3. Editing only the first row's note produces `ALTER TABLE app.events UPDATE note = 'changed' WHERE account_id = 7` through `edit-sql.ts:165`–168. Both events match. Deleting the first row through `buildDelete` at line 194 likewise produces a predicate covering both events.
4. Expected behavior is either a proven unique target or editing disabled; the selected first row is not a unique target in this table.

**Fix:** Keep ClickHouse index/primary-key metadata for display, but stop treating it as an enforced unique identifier. Disable row-targeted ClickHouse edits/deletes by default unless the product introduces an explicit, verified identity mechanism. A check of duplicate keys only within the currently loaded page is insufficient because another matching row can be outside it. Copy-as-INSERT can remain available since it does not target existing rows.

**Regression:** Use the two-row MergeTree fixture above and assert that row editing/deletion is unavailable and no mutation is sent. Include a paginated/filtered result showing only one of the matching rows, so a page-local uniqueness check cannot accidentally pass.

### C2-03 [major] Auto-stash drops staged selections, including after a failed checkout — `crates/otto-git/src/local.rs:3477`

**What:** Auto-stash restores with `git stash pop` without `--index`. It preserves worktree content in the ordinary successful case but does not restore which tracked changes were staged.

**Intended:** The failed-switch contract immediately above `checkout_autostash` explicitly says the tree is restored exactly as it was. A failed branch switch should not destroy the user's staged/unstaged partition.

**Why it matters:** Users who carefully staged individual hunks lose that selection when a branch switch fails, and can subsequently create a different commit than intended. The same issue occurs on successful auto-stashed switches.

**Evidence:** Confirmed, hand-traced command semantics. Start with tracked `a.txt`: HEAD contains `base`, index contains `staged`, worktree contains `unstaged`. Request `/repos/{id}/checkout` with `auto_stash:true` and an absent branch. `crates/otto-git/src/http.rs:1889` invokes the auto-stash path.

1. `checkout_autostash` stores index and worktree in its stash at `local.rs:2485` and checkout fails at line 2516.
2. Failure recovery calls `pop_stash_sha` at line 2522.
3. `pop_stash_sha` at line 3477 runs `stash pop <selector>` without `--index`. Ordinary stash application restores the worktree difference but does not reinstate the saved index state. After the failure, the tracked file is unstaged instead of partially staged.
4. The successful switch path has its own identical omission at `local.rs:2537`. The expected index content is `staged`; the actual index remains at the checked-out commit's version.

**Fix:** Restore auto-stashes with index restoration, preserving conflicts and retained-stash reporting. Cover both the direct successful-switch pop and the shared rollback helper; an `--index` restoration failure must retain the stash and report the recovery state rather than silently retrying without index preservation. Ordinary user-requested stash-pop semantics can be kept separate if needed.

**Regression:** In an isolated disposable repository, partially stage a tracked file, snapshot cached and uncached diffs, attempt an auto-stashed switch to an absent branch, and assert both diffs remain byte-identical afterward. Repeat with a successful switch to a branch at the same commit and with mixed staged-only/unstaged-only files.

### C2-04 [major] Concurrent API sends overwrite unrelated script variables — `ui/src/lib/stores/apiClient.svelte.ts:1428`

**What:** Each request snapshots all runtime variables and later replaces the entire workspace variable map with its script output. An older request can undo another request's variable writes even when its own script never changes variables.

**Intended:** Independent request tabs retain their own in-flight execution, while shared runtime variables preserve unrelated writes. Script `set`/`unset` operations should affect the keys they actually modify.

**Why it matters:** Concurrent login/test requests can erase or revert tokens and IDs produced by another request. Later `{{variable}}` substitutions use stale values despite both sends succeeding.

**Evidence:** Confirmed, hand-traced interleaving.

1. Begin with runtime variables `{token:'old'}`. Start a slow request A whose post-response script is only `console.log('done')`. Line 1364 snapshots `{token:'old'}`.
2. While A is pending, send request B from a different tab with post-response script `pm.variables.set('token', 'new')`. Per-tab ownership explicitly permits both sends (lines 1353–1361). `scripts.ts:60` mutates B's own map, and B completes, publishing `{token:'new'}` at line 1428.
3. A completes. Its worker receives A's original map at line 1423. `ui/src/lib/api/scriptWorker.ts:12` returns the whole map, unchanged by the logging script.
4. Line 1428 replaces the current map with `{token:'old'}`. A never requested a token write, but B's update is lost. The pre-request publication at line 1378 has the same whole-map overwrite pattern. A manual variable edit during the request is also vulnerable.

**Fix:** Return a script write set (set/unset operations) from the worker and apply only those operations to the current workspace map. Preserve the per-request snapshot for that request's own substitutions, and define ordering/conflict behavior when two scripts intentionally write the same key. Do not publish an unchanged full snapshot.

**Regression:** Control completion order for two tab sends. Let B set the token before A's read-only post script completes; the token must remain `new`. Also test disjoint-key writes, unset of one key while another is added, and a manual variable edit during a pending request.

## Coverage and limits

- Traced SQL simple SELECT rejection, PK resolution, generated updates/deletes, reviewed execution handoff, and ClickHouse catalog key publication. Did not exhaustively review every engine's SQL parser, wire conversion, import/export, masking, or native access-control implementation.
- Read Git checkout, auto-stash, stage/unstage, discard classification, and commit-only behavior. No Git commands that modify a repository were run. Conflict resolver reconciliation was sampled; external CLI concurrency and all merge/rebase recovery paths remain unverified.
- Traced API request snapshots, per-tab ownership, saved-draft adoption, and worker variable publication. Did not inspect the complete backend HTTP transport, OAuth, automation scheduler, or collection import format matrix.
- Broker group reset UI captures the selected cluster/group and checks selection after confirmation; preview responses are guarded against stale input. Read the reset service and Kafka offset resolver. No live broker/ACL/timestamp tests were run, and Kafka protocol/proxy/replay correctness is outside this pass's verified coverage.
- Sampled SFTP navigation and transfer initiation. SSH tunnel lifecycle, remote filesystem behavior, and database connection retry behavior were not exercised.
- No performance, security, architecture, or styling conclusions are implied by this correctness-only review.
