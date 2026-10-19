# E14-state — iteration 01

**Verdict: Block.** Five source-confirmed findings: two high-priority/current-path defects, two medium-priority defects, and one low-priority configuration edge. **Observed scope score: 7.5/10**, not 9.8; this is an engineering judgment of the inspected paths, not a whole-product score or an executed verification result.

Reviewer: E14-state. Date: 2026-10-18. Branch: `fix/db-parity-session-consistency` (PR111 context supplied by coordinator). HEAD at start and report preparation: `c009da20cc113c8fb79f3bf0600442d0c6afa358`. Review covers existing subsystem behavior, not only changed lines. Concurrent edits were present in git, sessions, workflows and swarm; none were modified by this reviewer.

Read AGENTS.md and correctness-review, test-review, architecture-review (including design vocabulary), and performance-review skills. Evidence below is **hand-traced source**, not an executed reproduction. No Cargo, builds, tests, Git mutations, live data operations or production edits were performed. The only filesystem write is this report. Early findings were delivered with reviewer ID to root session `01M57PYK75FC1F2AFS4C73BBF7` using `otto_send_message`.

## Findings

### E14-01 — P1 / blocker: a rejected connection save persists only part of the new profile

- **Location:** `crates/otto-state/src/connections.rs:179-233`, especially the separately committed section update at 211-217; caller `crates/otto-connections/src/service.rs:573-628`; route `crates/otto-connections/src/http.rs:435-455`.
- **Intent:** one profile save returns the updated connection or leaves the prior configuration intact. Target identity, credentials and environment protections must describe the same saved configuration.
- **Concrete input/trace:** begin with an existing connection in `dev`, `read_only=false`. An authorized root PATCH supplies a valid new target in `params`, a new name, `environment=prod`, `read_only=true`, and a section ID that was deleted while the form was open. The service validates connection parameters but never resolves the section. The repository independently commits name, params, optional secret reference and first command. Setting `section_id` violates its FK and returns an error. The later environment/read-only UPDATEs never execute.
- **Actual versus expected:** the request fails, but GET returns the new target/name with the old protection settings. Expected: reject the complete edit or commit all fields together. This is not a hypothetical disk error; a stale section ID deterministically reaches the failing SQL with foreign keys enabled by `db.rs:39`.
- **Related durability seam:** `service.rs:608-614` overwrites an existing Keychain secret before the database operation. A database rollback alone cannot restore that old credential. A failed save can already change it.
- **Fix direction:** make the entire repository patch one SQL statement or one `DbPool::begin()` transaction. For credential replacement, write a fresh secret reference, atomically switch the DB reference with the other fields, clean up the new secret on failure, and retire the superseded reference after success. Keep existing root capabilities.
- **Required regression:** with production FK settings, submit valid changed params plus an absent section ID and assert every persisted field equals its pre-request value. Include an existing-secret replacement and assert the old secret remains usable after rejection. A success case should atomically apply target and protections. No such test was executed here.
- **Ownership:** coordinate state/connections changes with the connections reviewer; no edit has been made.

### E14-02 — P1 / blocker: concurrent section moves create a durable cycle and a later move spins indefinitely

- **Location:** `crates/otto-connections/src/service.rs:241-268`; `crates/otto-state/src/connection_sections.rs:75-85`; route `crates/otto-connections/src/http.rs:1089-1104`.
- **Intent:** reparenting explicitly promises to reject cycles.
- **Concrete interleaving:** sections A and B are roots. Request 1 moves A under B; request 2 moves B under A. Both `list_all()` calls at service line 254 finish before either update. Each ancestor walk sees a root and permits the move. The two independent UPDATEs then commit A.parent=B and B.parent=A. The FK permits both because each parent exists.
- **Actual versus expected:** the stored section tree is cyclic, whereas at most one move should succeed. A subsequent request moving unrelated root C under A repeatedly walks A → B → A in the synchronous `while` loop at 256-265. Neither ancestor equals C and there is no visited set or await; the request never completes and occupies a runtime worker.
- **Fix direction:** put ancestor validation and the update behind one state-repository operation using `BEGIN IMMEDIATE`, so the second request checks the first committed move. Bound traversal with a visited set and return a useful conflict/error on an already-corrupt cycle. This is a concrete seam issue: validation in a caller cannot preserve the repository invariant across concurrent callers.
- **Required regression:** synchronize two opposing moves before the write point and assert exactly one is rejected, then verify the resulting graph is acyclic. Separately seed a cycle and assert moving C under it terminates with an error. The current repository reparent test (`connection_sections.rs:264-268`) exercises sequential valid moves and cannot expose this interleaving.

### E14-03 — P2 / major: product revision allocation is not serialized

- **Location:** `crates/otto-state/src/product.rs:859-891`; consumers `product.rs:933-946` (`synced_page_version`), `product.rs:952-1003` (latest-version queries); schema `crates/otto-state/migrations/0024_product.sql:25-39`.
- **Intent:** `version_no` orders a story's revision history and determines its latest source, plan, suggested body and synchronized remote page version.
- **Concrete interleaving:** story S has maximum revision 4. Two independent saves both finish the SELECT at line 864 before either INSERT. Both allocate 5 and both INSERT successfully: row IDs differ and no migration adds a unique `(story_id, version_no)` constraint. This is reachable through independently handled refresh requests (`otto-product/src/http.rs:545-553`, `service.rs:339-370`), as well as agent/workflow version producers.
- **Actual versus expected:** two different revisions carry version 5. Queries order only by `version_no DESC`, so they no longer distinguish the newer committed revision and may return an earlier body or synchronization base. Expected numbers 5 and 6, with stable latest selection.
- **Fix direction:** allocate and insert under one immediate transaction (or a single INSERT…SELECT) in the repository, not separate locks in each producer. Consider an append-only uniqueness migration only with a deliberate repair strategy for existing duplicates; adding a unique index blindly would brick affected installs.
- **Required regression:** concurrent same-story inserts on the real split file-backed pool must produce distinct increasing numbers and correct latest selection. Include callers of the same kind so a kind filter cannot hide the tie. Assert bodies/IDs, not merely that both operations returned success.

### E14-04 — P2 / major, legacy-upgrade population: migration 0013 deletes improvement edit history

- **Location:** `crates/otto-state/migrations/0013_improvement_trigger_live.sql:21-23`; child FK `crates/otto-state/migrations/0009_self_improvement.sql:18-20`; FK enablement `crates/otto-state/src/db.rs:39`.
- **Concrete input/trace:** upgrade a populated database at migration 12 with improvement run R and edit E referencing R. Migration 13 copies parent R into `improvement_runs_new`, then drops the original parent table. With FK enforcement on, DROP performs the parent deletion and the child's `ON DELETE CASCADE` deletes E. Renaming the copied parent back restores R but cannot restore E. The migration comment stating that no FK action fires because child rows are untouched is incorrect.
- **Actual versus expected:** saved edit history, including before/after content used for rollback, disappears while the run remains. Expected: widening the trigger CHECK preserves both parent and child rows. Severity is scoped: current databases already past 13 do not execute this again, and fresh databases have no rows to lose.
- **Fix direction:** preserve children around this known old rebuild in an upgrade compatibility/preflight path; do not change an applied migration's checksum or assume a later migration can reconstruct deleted content. For already-affected databases, recovery needs a pre-upgrade backup, if available.
- **Required regression:** apply migrations through 12, seed a real workspace/run/edit with FK enforcement, upgrade through 13 using production migration semantics, and assert the complete child row survives. `tests/migration_compat.rs:23` and `tests/migration_rebuilds.rs:20` baseline at 176; their schema/index checks do not cover this populated historic upgrade. The populated 185 account-rebuild test is a good pattern to extend.

### E14-05 — P3 / minor: an excessive retention window permanently terminates the hourly worker

- **Location:** `crates/otto-state/src/retention.rs:144-154,223`; ingress `crates/otto-server/src/routes/settings.rs:24-58`; worker `crates/otto-server/src/boot/tasks.rs:330-390`.
- **Concrete input/trace:** an authorized settings save sets `data_retention.work_events_days` to `9223372036854775807` (JSON integer `i64::MAX`). The settings endpoint stores it without retention validation. Deserialization succeeds; floors preserve the value. `Duration::days(days)` at line 223 panics instead of returning a Result. The local dependency source confirms this (`chrono-0.4.45/src/time_delta.rs:137-138`, `expect(try_days(...), ...)`). Smaller but still excessive durations can also overflow DateTime subtraction.
- **Actual versus expected:** the detached task exits; its `match repo.prune(...).await` cannot catch a panic. Retention, expired-auth purge and hourly SQLite maintenance on that same loop stop until restart. Correcting the setting alone cannot revive the task. Expected: reject/clamp an unrepresentable window and keep maintenance alive.
- **Fix direction:** validate a documented maximum or use both checked duration construction and checked date subtraction. Return a normal configuration error and keep the worker alive; no reduction of root authority is necessary.
- **Required regression:** exercise `i64::MAX` and a duration representable as TimeDelta but not as a date subtraction, plus valid boundaries. Assert a normal error or safe clamp and a subsequent maintenance pass. Existing floor/default tests do not exercise the upper bound. Low priority reflects the exceptional input, despite the persistent effect.

## Coverage, positive evidence and cost assessment

- Inspected the pool router and transaction APIs, bootstrap/migration repair/snapshot code, compaction swap/rollback/confirmation and caller ordering, maintenance, retention policies and delete algorithms, run-history retention and its tests, rebuild/additivity tests, representative migration rebuilds and FK consumers, connection and section repositories/services/routes, workspace settings transactions, product revision writers/readers, settings cache, canvas snapshot/pruning transactions, and selected database-change CAS operations. Sampled projects/workbench and state-archive transaction boundaries; these are not complete audits of those modules.
- `DbPool::begin()` and its generic Acquire implementation both use `BEGIN IMMEDIATE`; snapshot reads use a separate read-only pool. This avoids the common deferred read→write snapshot-upgrade failure. Explicit transactions in database-change mutations combine CAS state transitions with events; workspace `mutate_settings` serializes its JSON read/modify/write. These are useful local patterns for fixing E14-01 through E14-03.
- Pre-migration backups use `VACUUM INTO`, rather than copying an active database file without its WAL. Offline compaction verifies schema and table counts and retains the previous database until open/migration succeeds. I did not establish a normal-path data-loss defect in that flow. Failure/power-loss ordering still needs fault-injection validation; it is not a clean durability certification.
- Retention defaults keep run history and room transcripts unless opted in. Deletes are generally bounded to 1,000 rows with a 25 ms pause. Swarm run deletion and spend rollup share one transaction. Run-history tests check preserved spending and terminal/young/live cases, but their in-memory fixture disables FKs and uses a single pool; they do not validate production split-pool concurrency or cascade accounting.
- Cost sizing: connection save performs up to seven independent commits plus a read for one logical edit; a single UPDATE would reduce this to one commit and fix atomicity. Retention cost is driven by old rows and candidate work items, with per-item reads and batches; no wall-clock throughput claim is made. No standalone performance blocker or broad architectural rewrite is justified from this static pass.

## Residual boundaries and next round

- No tests, benchmark, SQL replay, daemon calls against state, fault injection, crash/restart or UI exercise was allowed or performed. All repros above are specifications for the allocated validation round, not claims of passing/failing executions.
- Not every method in the 80+ state modules or all 187 migrations was read line by line. Sessions, git, workflows, automation, assistant and swarm consumer state machines remain with their assigned reviewers. This report does not certify them, FTS consistency in every index, all retention tables, multi-process compaction exclusion, archive filesystem crash recovery, or all privileged restore paths.
- The E16 credential-routing request is outside E14's assignment and was not investigated here.
- Existing privileged/root abilities are preserved by every proposed fix. No permission-gating redesign is requested.
- Fix ownership and test resources are pending coordinator allocation. E14 remains available for state fixes and cross-review. Re-score only after concrete repairs, targeted regressions, and inspection of concurrent final source; 9.8 is not warranted now.

## Allocated fix round and second-pass review

The coordinator subsequently allocated E14-01/02 state repositories, E14-03 revision allocation, and E14-05 bounds to E14. E09 owns all `otto-connections/src/service.rs` changes; E12 owns watcher/attachment persistence, E13 review persistence, E20 workspace settings, and E11 canvas. No migration number is needed by E14; E10/E11/E18 also reported no main-state migration requirement. Existing migrations remain untouched.

Implemented (verification in progress):

- `ConnectionsRepo::update` now begins one immediate transaction, applies every supplied field, decodes its returned row under that transaction and commits once. FK rejection rolls back earlier field writes; optional/nullable PATCH semantics stay intact.
- `ConnectionSectionsRepo::reparent` owns existence/ancestor validation and the update under the same immediate transaction. A visited set rejects already-corrupt ancestor chains. Setting parent to None can repair an old cycle. E09 replaced the unsafe service-side ancestor walk with repository delegation.
- `ProductRepo::add_version` allocates its number, inserts and reads its result under one immediate transaction. No uniqueness migration or historical renumbering is attempted; already-persisted duplicate revision numbers remain a residual data-repair question.
- Retention uses checked duration construction AND checked date subtraction. Every configured cutoff is validated before any deletes; errors return through the normal worker error path. Disabled retention remains a no-op. Root can still configure retention normally.
- E09 accepted staged unique secret references with cleanup on rejection/success. Second-pass review additionally identified the need to reread the row after acquiring the credential mutex to avoid leaking a concurrently superseded reference; E09 incorporated this. Delete's old secret-before-row order was also sent to E09 for correction.

`crates/otto-state/tests/atomic_updates.rs` adds production file-backed split-pool regressions for rejected connection patches/all fields/nullable clears, opposing section moves/missing parents/self/corrupt cycles/repair, 32 concurrent story revisions and latest body, and oversized retention settings with no partial pruning and successful subsequent maintenance. These tests were written before implementation, but the Rust red phase could not be run because the shared Cargo slot was unavailable. Rustfmt succeeded on E14-exclusive files. The coordinator has now granted the slot; focused regression target, package library tests and all-target clippy are queued/running. No passing Rust result is claimed yet.

### Historic 0013 preservation proposal (not implemented)

An isolated Python SQLite replay used the exact migrations 0001–0013 with FK enforcement and a populated run/edit. Observed: **1 edit before → 0 after** unmodified 0013. A separate in-memory prototype copied the edit into an independent preservation table before the migration and restored it afterward; all original column values matched byte-for-byte and `foreign_key_check` was clean. Neither experiment touched a user database or any migration file.

The safe production direction is a narrowly version-gated bootstrap compatibility shim, not a numbered migration after the destructive one:

1. After the existing pre-migration snapshot, detect that 0013 is pending and the old improvement tables exist. In a committed immediate transaction, copy all child columns to a dedicated durable preservation table with no parent FK. Do not proceed with migration if preservation fails.
2. Make preparation idempotent: if an unfinished preservation table exists, retain it rather than replacing it with the now-empty live table. A crash after 0013 but before later migrations must not erase the rescue copy on restart.
3. Run the unchanged embedded migrator. After 0013 is applied, restore preserved rows and verify exact contents, parent references, and counts within one transaction. Treat conflicting IDs as an explicit recovery error; do not silently replace user rows. Drop the preservation table only in the same committed transaction as successful restore/verification. If later migrations fail, leave the durable rescue data for the next boot.
4. Add file-backed upgrade tests for pre13 populated data, fresh/current DB no-op, preparation rerun, interruption after 0013, and restore failure/retry. A dedicated bootstrap helper avoids spreading migration special cases into repositories.

**Already-lost historical edit data cannot be reconstructed from the surviving parent rows.** Recovery requires an intact prior backup or another original copy. An append-only migration today cannot recover bytes deleted by a past upgrade. This proposal therefore protects future upgrades of still-pre13 databases; it is not a claim that E14-04 is repaired in the current tree.

### First allocated verification slot — completed

- `cargo test -p otto-state --test atomic_updates -- --test-threads=2`: **4 passed, 0 failed** (1.61 s tests).
- `cargo test -p otto-state --lib -- --test-threads=2`: **462 passed, 0 failed, 2 ignored** (155.35 s). Ignored: `canvas::tests::canvas_large_scene_budget`, `workflows::tests::big_run_write_budget`. Includes E13 review repository tests.
- `cargo test -p otto-state --test workspace_settings_patch --test workspace_context -- --test-threads=2`: **workspace_context 4 passed; workspace_settings_patch 3 passed; 0 failed**.
- `cargo clippy -p otto-state --all-targets -- -D warnings`: **passed** (28.71 s).
- Scoped whitespace and rustfmt checks passed except an `add_version` commit-chain formatting correction, which was applied before clippy.
- Cargo slot released to root immediately after clippy. No Cargo process remains owned by E14.

E14 files at this checkpoint: `crates/otto-state/src/connections.rs`, `connection_sections.rs`, `product.rs` (`add_version` only), `retention.rs`; new `crates/otto-state/tests/atomic_updates.rs`; this report. E09 separately owns the coordinated connection-service changes and service tests. No commit, push, stash, revert or deployment occurred. The pre13 helper was subsequently approved and remains the only outstanding allocated implementation, followed by its separately allocated focused verification. No new sessions will be opened.

### Approved pre13 implementation — source frozen, focused checks pending

Root approved the narrow helper after the crash/idempotency review. Added `crates/otto-state/src/db/pre13.rs` and wired `prepare`/`restore` in `crates/otto-state/src/db.rs`, before/after the unchanged embedded migrator. No SQL migration was edited or added. Preparation and restoration failures stop startup with explicit diagnostics; preserved content is never logged. The helper commits a rescue table without FKs before migration, reuses an unfinished rescue table on retry, inserts only missing IDs, verifies all sixteen original columns with SQL EXCEPT, and drops the rescue table in the same transaction as verified restoration. Conflicting IDs or missing parents fail and retain the rescue copy.

Six new file-backed helper tests cover:

- Full production `db::open` upgrade of populated pre13 data, exact column preservation, FK integrity, and a repeated boot.
- Restart after preservation and after unmodified 0013, preserving the existing rescue copy instead of replacing it with the empty live table.
- Injected mid-restore SQL failure, rollback of earlier restored rows, conflicting existing row refusal, and a successful retry.
- Missing parent rejection without consuming the rescue copy.
- Fresh/current-schema no-op behavior.
- Injected preservation DDL failure stopping production bootstrap before migration 0013, followed by repair/retry.

The bootstrap change has passed scoped rustfmt and whitespace checks only so far. The earlier 462-test result predates this helper and must not be cited as its verification. Requested next slot: `cargo test -p otto-state --lib db::pre13::tests -- --test-threads=2`, existing `db::tests`, and all-target state clippy. Root independently reviews the helper before acceptance. No E14 Cargo process is running while waiting for that allocation. Existing historical loss is still unrecoverable without a prior copy; the shim protects still-pre13 upgrades and its own unfinished recovery records.

## Root verification of preservation helper

Independently read preparation, restoration, startup wiring and failure tests. Executed on the integrated working tree: `db::pre13::tests` **6 passed**, `db::tests` **12 passed**, and `cargo clippy -p otto-state --all-targets -- -D warnings` **passed**. The helper does not rewrite applied migration SQL and preserves rescue rows on conflicts. This closes the helper execution gap above; it does not replace the later workspace consumer gate.

## Ronaldo (`agy`) Allocated Cargo & Independent Verification Evidence (Iteration 2 Score: **9.8 / 10**)

- **Integration Suites (`otto-state`):**
  - Command: `cargo test -p otto-state --test atomic_updates --test workspace_context --test workspace_settings_patch` (log: `task-720.log:223-250`)
  - Result:
    - `atomic_updates`: **4 passed, 0 failed** (`1.55s`).
    - `workspace_context`: **4 passed, 0 failed** (`1.66s`).
    - `workspace_settings_patch`: **3 passed, 0 failed** (`1.17s`).
- **Lib & Server Consumer Suites (`otto-state`, `otto-server`):**
  - `db::pre13::tests` **6 passed**, `db::tests` **12 passed**, `reviews::tests` **16 passed** (`/tmp/otto-root-state-reviews.log`).
  - `routes::workspaces::tests::atomic_settings_patch_validates_before_writes_and_keeps_root_access`: **PASSED** (`/tmp/otto-root-server-focused-tests.log:67`, `74/74` passed, `0` failed, `0` ignored in `7.80s`).
- **Clippy Gate (`otto-state`):**
  - Command: `cargo clippy -p otto-state --all-targets -- -D warnings`
  - Result: **PASS (0 warnings)**.
- **Residual Boundary:** All `otto-state` lib/integration/clippy gates and the `otto-server` atomic settings patch route regression pass; historical pre-shim data loss on already-migrated databases remains unrecoverable without a prior backup, and the final coherent workspace gate remains pending.


## Root independent preservation review

Re-read `db/pre13.rs::prepare/restore` and `db.rs` wiring against migration0009. No new defect found in this bounded pass: preparation commits an FK-free rescue before the migrator; a pre-existing rescue is never overwritten; restoration checks non-null run/workspace parents, inserts only missing IDs, compares every preserved column with EXCEPT, then drops the rescue in the same transaction. A differing existing ID or missing parent returns before commit, leaving both live data and rescue intact. Read-pool publication follows successful restoration. Fresh/current databases skip the copy. This complements the previously executed six pre13 and twelve db regressions; it does not recover edits already lost by a past upgrade, or claim power-loss durability beyond SQLite's configured persistence guarantees. No production changes made in this review.
