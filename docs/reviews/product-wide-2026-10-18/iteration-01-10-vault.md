# E10-vault — iteration 01 engineering review

**Verdict: Block. One blocker and five major findings. Scoped engineering score: 7.4/10; 9.8 is not established.** All findings below are source-hand-traced, not runtime reproductions. No Cargo, builds, tests, production edits, or git mutations were performed. This report is the only file written.

Review date: 2026-10-18. Branch: `fix/db-parity-session-consistency`; observed HEAD `c009da20cc113c8fb79f3bf0600442d0c6afa358`. Other agents' live edits were present in sessions, git, workflows and later swarm; none were overwritten. This review covers existing subsystem behavior, not just the PR diff. Skills applied: correctness-review, test-review, architecture-review, performance-review.

## Confirmed findings

### E10-01 — Blocker: OKF index generation bypasses recoverable writes

**Location:** `crates/otto-vault/src/okf.rs:360-363`; caller `ui/src/modules/vault/vault.svelte.ts:1312-1323`, HTTP `crates/otto-vault/src/http.rs:383-391`. Compare normal writes at `crates/otto-vault/src/engine.rs:1254-1288`.

**Concrete trace:** Register a vault containing `services/a.md` and an existing manually authored `services/index.md` with unique explanatory content. Invoke Generate indexes. The generator builds replacement Markdown and directly calls `tokio::fs::write` on the existing index. The old content is overwritten without `prepare_revision`/`commit_revision`; no recovery entry for that previous content is created. This holds even without concurrency or any failure. A write failure after truncation additionally leaves a partial index. The route's regeneration semantics authorize replacement, but do not justify bypassing the product's existing recovery mechanism.

**Expected versus actual:** Generated replacement should retain a recoverable before-image and use atomic replacement; actual replacement can permanently discard the only copy of the old index. The operation also bypasses the vault write lock and publication fence.

**Fix direction:** Generate content independently, then use the normal guarded note writer with the captured current hash (empty hash for creation). Preserve per-file history and return/report conflicts rather than clobbering an intervening edit. Do not add restrictions on privileged roots.

**Required regression:** Start with unique handwritten content, generate, and retrieve a revision whose `before` equals that content; restore it successfully. Cover an intervening edit and an injected replacement failure. `tests/engine.rs:790-805` checks generated content only: it would still pass with all recovery safeguards absent, as they currently are.

### E10-02 — Major: empty folders disappear from the file tree

**Location:** `crates/otto-vault/src/engine.rs:1448-1458`, `crates/otto-vault/src/scan.rs:65-73`, `crates/otto-vault/src/index.rs:85-112,139-149`; caller `ui/src/modules/vault/vault.svelte.ts:1048-1052`.

**Concrete trace:** POST folder `{path:"new-folder"}` returns 204 after mkdir. The UI refreshes the tree. `dir()` serves only `VaultIndexes`; its directory entries are synthesized from indexed file ancestors. The filesystem walk records only notes/files and never an empty directory. Consequently `new-folder` is absent immediately, after an explicit rescan, and after engine recreation. Deleting the last file in a folder also removes its tree entry even though the folder still exists on disk.

**Expected versus actual:** A created directory must be available to select, rename, delete, or create a note within. It currently exists only on disk and cannot be selected in the refreshed tree.

**Fix direction:** Represent explicit directories in the scan and derived index, including hydration and removal. Keep directory child counts independent of whether the physical directory exists. Updating only the UI or adding a rescan cannot fix the missing representation.

**Required regression:** Create/list an empty directory, rescan/recreate engine/list again, external mkdir, remove last child, and external rmdir. Existing folder-rename coverage at `tests/engine.rs:477` contains notes and cannot detect this case.

### E10-03 — Major: a transient FTS probe failure permanently disables body search and leaves holes after restart

**Location:** `crates/otto-vault/src/engine.rs:166-175`; `crates/otto-vault/src/store.rs:1221-1232,1235-1260,551-555`; incremental scan `engine.rs:715-727` and `scan.rs:161-179`.

**Concrete trace:** On the first FTS probe, inject one transient database error in table creation or map initialization. `ensure_fts` returns false for any error; `fts_ready` stores terminal value 2. Subsequent scans and writes successfully publish note metadata with `fts=false`. Search then uses LIKE over title/path/description, so a word occurring only in the body is absent. Clearing the original database failure does not cause a new probe. After restart a successful probe merely creates the FTS structures; it does not backfill notes. An unchanged note's size/mtime match its stored signature, so even explicit rescan skips it and its body remains absent from FTS.

**Expected versus actual:** A temporary DB failure should recover, and the derived search index should be rebuildable. Actual behavior degrades indefinitely and can survive restart for unchanged notes.

**Fix direction:** Distinguish unsupported FTS from transient errors, single-flight the probe, retry transient failures, and reconcile/backfill missing or stale FTS rows when availability returns. `otto-memory/src/service.rs:118-155` already provides a useful local retry pattern, but Vault also needs body re-preparation/backfill.

**Required regression:** One-shot probe failure, successful note indexing during fallback, recovery, body-only search, engine recreation, explicit rescan. Assert recovered results and correct FTS row contents, not only metadata presence. No equivalent Vault transient-probe recovery test was found in the reviewed engine/performance tests.

### E10-04 — Major: watcher changes during a background scan are dropped

**Location:** `crates/otto-vault/src/engine.rs:589-598,658-664`; producer `crates/otto-vault/src/watch.rs:226-235`; watched freshness interval `watch.rs:28`.

**Concrete interleaving:** A background scan is in `pending_scans`, has walked the tree and processed `a.md`, and is still processing other files. An external editor changes `a.md` (or adds `new.md` after the walk). The watcher consumes/debounces that event, detects a signature mismatch, and calls `kick_scan`. Because the id is already pending, the call returns without recording follow-up work. The first scan finishes and sets last-scan time to now, then removes the pending id. No queued event remains. Search/tree still reflect the earlier snapshot; the healthy watcher stretches the read-driven fallback to over 600 seconds.

**Expected versus actual:** Coalescing should ensure at least one scan after the latest observed change. It currently discards that obligation. This is particularly reachable during large imports, where a scan outlasts the 300 ms debounce.

**Fix direction:** Maintain a dirty/rerun generation while a scan is pending, and drain it before clearing pending state. Coordinate the final clear atomically with incoming kicks; do not solve this by restoring frequent whole-vault polling.

**Required regression:** Barrier-controlled background scan after its walk; add a file; deliver a watcher kick while pending; resume; require a second pass and visibility without another event or staleness timeout. `performance_tests.rs:1255-1266` verifies an idle external edit, not this overlap.

### E10-05 — Major: failed memory mirror exports are silently lost and retry cannot repair them

**Location:** `crates/otto-memory/src/service.rs:185-196`; `crates/otto-memory/src/vault.rs:154-160`; production enablement `crates/otto-server/src/boot/build.rs:337-341`; dedup `crates/otto-state/src/memory.rs:478-483`.

**Concrete trace:** Enable `OTTO_MEMORY_VAULT_DIR` pointing to an unavailable/unwritable destination. Save a shared memory. The database transaction commits; `VaultWriter::write` fails; `let _` discards the error, and save returns success. Repair the destination and repeat the identical save. The repository returns `SaveOutcome::Existing`; `outcome.is_new()` is false, so no export is attempted. The memory never appears in the shareable folder through that retry.

**Expected versus actual:** An enabled write-through mirror must expose export failure and support eventual repair. Actual failure is silent, with no durable retry obligation, and a natural idempotent retry does nothing.

**Fix direction:** Persist export status/outbox work after the DB commit, retry idempotently, and expose a useful partial-success/export status. At minimum, log/report failure and allow an existing record's missing export to be repaired. Use atomic file replacement for export. Moving disk I/O off the async runtime is appropriate in that same bounded export worker.

**Required regression:** Destination failure, successful DB save, destination recovery, same-item retry/reconciliation, exactly one complete exported file. `otto-memory/tests/vault.rs:31-57` only exercises a writable temporary directory; the tests at line 123 guard initial private-memory omission, not failed-export recovery.

### E10-06 — Major: concurrent cross-workspace registration breaks global root uniqueness and write serialization

**Location:** `crates/otto-vault/src/store.rs:88-119`; schema `crates/otto-state/migrations/0105_vault_docs.sql:8-17`; write locks `crates/otto-vault/src/engine.rs:579-586`.

**Concrete interleaving:** Two permitted registrations from different workspaces canonicalize the same existing root. Both execute the duplicate SELECT before either INSERT. Both see no row. The inserts succeed because the database constraint is only `(ws_id, root_path)`, producing two vault ids for the same physical root. The documented global-root invariant (`docs/contracts/api.md:3241-3244`) is violated. Writes lock by vault id, so the two ids also bypass each other's serialization: two matching-hash updates can both read the same before-image and replace the same file.

**Expected versus actual:** Exactly one registration should succeed; the other should receive Conflict. Different workspace provenance currently permits duplicate handles and separate locks over the same files.

**Fix direction:** Make duplicate check and insert one serialized write transaction, or use an equivalent canonical-root admission lock. Preserve pre-existing registrations; do not delete legacy rows or require a destructive uniqueness migration. Consider keying filesystem mutation locks by canonical root as defense for legacy duplicate rows.

**Required regression:** Barrier both registrations after duplicate check, release, assert one success/one Conflict and one root row. Cover different workspaces specifically; a same-workspace test is protected by the existing SQL constraint and would miss the defect. Verify writes through any supported legacy duplicate handles cannot both win a shared hash.

## Coverage and lens assessment

- **Persistence and recovery:** Read normal note/artifact writes, hash checks, atomic replacement, history preparation/coalescing/commit/list/restore, trash and rename paths, generation publication and repair. Atomic same-directory replacement, persisted before-images on normal writes, checksummed revision reads, NOREPLACE moves and explicit post-mutation scan error handling are meaningful strengths. E10-01 is a concrete write-boundary exception, not a request to redesign working recovery.
- **Synchronization and search:** Read scanner diff/prune logic, preparation size limits and cancellation, cache hydration/epochs, watcher debounce/overflow/signature checks, FTS creation/index/search, filter-before-limit and link reconciliation. E10-03/04 prevent a high reliability score.
- **Memory:** Read save/dedup/export/reindex and update/forget/search paths, relevant production construction, HTTP entry points and selected governance code. Confirmed the mirror is production-configurable, not a dead utility.
- **Tests:** Inspected engine integration tests, targeted performance/publication/watcher tests, memory vault/integrity tests and relevant inline scan/preparation/recovery tests. Existing coverage materially guards internal CAS concurrency, case-only rename, incomplete walks, history checksums and bounded preparation. The six regression scenarios above are needed before claiming fixes. Tests were not executed.
- **Architecture:** The existing guarded writer already supplies the needed seam for generated indexes. Reusing it fixes locality: future durability/concurrency changes need one implementation instead of a raw-write exception. Explicit directory representation is needed at the index boundary. No separate architecture finding is counted for the same defects.
- **Performance:** Preparation bounds admission at 4/running work at 2; scanned bodies batch at 200 notes/16 MiB; search limits at 200 and filters precede LIMIT; cached directory reads avoid repeated whole-vault SQL reads. Rename still performs one incoming-source query per moved file, and OKF index generation scans its directory set per directory, but these are user-triggered paths and no production latency measurement was authorized. No independent performance blocker is claimed from those costs alone.

## Residual boundaries and next pass

This is not exhaustive certification of the approximately 12.5k source lines in these two crates. Parsing dialect corners, graph encoding/large-vault rendering, every OKF validation rule, remote memory/governance parity, memory mirror update/forget/reindex identity semantics, and docs-agent orchestration were not fully adjudicated. UI was inspected only to establish callers, not visually reviewed. No native FSEvents experiment, power-loss test, permission/disk-full fault injection, or load benchmark ran. External-editor races inside the hash-check-to-rename window remain a follow-up boundary; the six findings above do not rely on claiming that race is solved or unsolvable.

The source review supports a block on recoverability and fixes for the five major gaps; it does not establish runtime incidence rates. Re-review the relevant source snapshot after ownership-coordinated fixes, execute deterministic regressions under the root's test allocation, then reassess the score. E10 is available for owned fix work and cross-review. E16's credential-token investigation is outside this assignment.

## Allocated fix pass — implementation and verification pending

Root allocated all six backend findings to E10. No existing migration was edited and no new migration is needed; E14 was notified. No UI changes, commits, pushes, or deployments were made. Root separately authorized surgical Vault/memory contract and feature-guide prose.

- **E10-01 implemented:** OKF generation captures each existing file hash and calls the guarded note writer; existing content is revisioned, intervening edits conflict, and history preparation failure leaves the original untouched. Tests cover revision retrieval/restore, a barrier-controlled concurrent edit, and failed history storage.
- **E10-02 implemented:** directory responses merge warm file metadata with a shallow off-runtime physical-directory enumeration, so empty directories need no SQL migration and are visible after restart. Complete scans retain a directory-existence snapshot for watcher checks and bump generation on changes; creation publishes generation immediately. Tests cover create/nested directories, last-child deletion, restart, external mkdir/rmdir, and watcher generation changes. The metadata-only 20k-file test now creates its 200 real directories to match the physical-folder contract.
- **E10-03 implemented:** single-flight FTS probing distinguishes unavailable modules from transient errors and retries the latter after backoff. A transient fallback write persists a stale signature for later preparation; successful initialization marks absent/dangling FTS mappings for re-preparation, including previous-process rows. Unsupported FTS does not continually invalidate signatures. Tests cover changed/new body recovery, old-body removal, restart backfill, and no repeated parsing when unsupported.
- **E10-04 implemented:** pending scans retain a rerun bit under the same mutex used to clear pending state. A barrier-controlled test adds a file after a walk and kicks while pending, requiring publication without another event.
- **E10-05 implemented:** mirror export runs off-runtime and reports that a committed DB save needs an export retry. Duplicate saves repair missing files through no-replace atomic publication, preserve existing external edits, and reject non-file destinations. Tests cover deterministic export failure/retry, concurrent duplicate exports, and directory collision.
- **E10-06 implemented:** `BEGIN IMMEDIATE` now encloses duplicate-root lookup and insert. Root-keyed write locks also protect pre-existing duplicate handles. Tests pause after the first lookup to assert another workspace cannot insert, and exercise matching-hash writes through legacy duplicate ids.

**Independent pass:** found and addressed empty-folder watcher/generation propagation, unsupported-FTS reparsing, and non-file export destinations. No other agent's changes were overwritten. Added matching contract prose without touching API/gRPC or workflow sections.

**Actual verification so far:** owned-source rustfmt parsing/formatting succeeded (one subsequent formatting check flagged a newly added test; it was then formatted); an isolated in-memory Python SQLite check executed the exact FTS repair SQL and verified missing rows and dangling maps are marked while an intact row is preserved. `git diff --check` for owned changes passed. These are not Rust compilation or test results. Root has queued the requested package test/clippy commands; no Cargo command has run in this fix pass. Score remains unverified and no 9.8 claim is made pending execution and cross-review.

## Source-frozen handoff to root / Ronaldo / Pele

All six allocated source fixes and their regressions are implemented; implementation is frozen for coordinated validation. No new assignment or session was opened. All edits are preserved; nothing was reverted, committed, stashed, pushed, or deployed.

**Exact E10 files:**

- `crates/otto-vault/src/engine.rs`
- `crates/otto-vault/src/index.rs`
- `crates/otto-vault/src/store.rs`
- `crates/otto-vault/src/scan.rs`
- `crates/otto-vault/src/watch.rs`
- `crates/otto-vault/src/okf.rs`
- `crates/otto-vault/src/recovery.rs` (root-keyed write-lock call only)
- `crates/otto-vault/src/performance_tests.rs` (physical directories for the existing metadata scale fixture)
- `crates/otto-vault/src/durability_tests.rs` (new regression module)
- `crates/otto-memory/src/service.rs`
- `crates/otto-memory/src/vault.rs`
- `crates/otto-memory/tests/vault.rs`
- `docs/contracts/api.md` (only the memory mirror note and Vault dir/index-generation rows; other changes in this shared file belong to other reviewers)
- `docs/features/vault.md` (freshness, empty folders, generation recovery/conflicts, memory mirror retry)
- This review report.

**Actual checks:** final owned-source `rustfmt --edition 2021 --check` passed; owned/shared-doc whitespace check passed; isolated SQLite repair-query assertions passed. No Rust tests, compilation, or clippy ran. The last source edit after the formatting check only made the scan-loop mutex lifetime explicit with a scoped `rerun` boolean; include it in the next formatting check.

**Queued commands for the shared slot:** `cargo test -p otto-vault -p otto-memory`, then `cargo clippy -p otto-vault -p otto-memory --all-targets -- -D warnings`. Run formatting checks for the listed Rust files as well. Root should schedule any required affected-consumer/workspace gate after these package checks. No full browser suite is requested; no UI code changed.

**Pending issues:** compile/test/lint outcomes are unknown; fix any failures before accepting the six findings as closed. In particular validate the new pause-hook concurrent tests, native directory fsync/atomic mirror publication, watcher generation behavior, and the existing scale suite. No migration is required; E14 was informed. Reported broader residual boundaries above remain outside this six-fix allocation. Score is not raised pending verification/cross-review.

**Cargo ownership:** never acquired; no Cargo/build/test command or background process is running for E10. The shared slot is not held by this session.

## Ronaldo (`agy`) Allocated Cargo & Independent Verification Evidence (Iteration 2 Score: **9.8 / 10**)

- **Lib Unit Tests (`otto-vault`, `otto-memory`):**
  - Command: `/tmp/otto-nextest-0.9.146/cargo-nextest run -p otto-vault -p otto-memory --lib` (log: `task-697.log`) + `cargo test -p otto-vault -p otto-memory --lib` (log: `task-720.log`)
  - Result:
    - `otto-vault` (`lib`): **91 passed, 0 failed, 5 ignored** (scale gates), including all 6 `durability_tests` regressions (OKF guarded write + revision conflict, empty physical directories across restart/watcher, FTS transient-vs-unsupported recovery, pending-scan rerun latch, and `BEGIN IMMEDIATE` duplicate-root serialization).
    - `otto-memory` (`lib`): **8 passed, 0 failed**.
- **Integration Suites (`otto-vault`, `otto-memory`):**
  - Command: `cargo test -p otto-memory --test vault && cargo test -p otto-vault --test engine` (log: `task-720.log:370-420`)
  - Result:
    - `otto-memory --test vault`: **7 passed, 0 failed** (`2.22s`), covering off-runtime mirror export, deterministic export failure/retry, concurrent duplicate exports, and non-file destination rejection.
    - `otto-vault --test engine`: **32 passed, 0 failed** (`10.49s`).
- **Clippy Gate (`otto-vault`, `otto-memory`):**
  - Command: `cargo clippy -p otto-vault -p otto-memory --all-targets -- -D warnings`
  - Result: **PASS (0 warnings)**.
- **Residual Boundary:** All `otto-vault` and `otto-memory` unit, integration, and clippy gates pass, and `otto-server --tests` compilation (`/tmp/otto-product-wide-server-check.log`) passed; pending only the final coherent workspace gate.
