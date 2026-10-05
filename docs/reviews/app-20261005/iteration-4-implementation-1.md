# Iteration 4 implementation — partition 1

Scope: R4-C1-01/02, P1-R4-01 and R4-U1-01. Source repairs are in progress; no final execution or 9.8 acceptance is claimed. Root owns builds, unit/browser runs and combined gates. Claude owns visual/a11y/copy work, ConversationView search disclosure, TabBar and preview retry; those files were not edited here.

## Reproductions and chosen repairs

Root executed the initial existing-function/real-store regressions: **30 tests, 23 passing, 7 failing**, `/tmp/otto-review05-role1-red.log`. Failures reproduced History working/live classification, stale inactive restart, warmed view preference, mixed-batch dismissal, retained collapsed children, unbounded 1,200-turn child history, and late collapsed-body publication. The History no-restart assertion now runs independently before its label assertion; newer child navigation is mandatory, not optional.

Root then ran `cargo test -p otto-sessions --test isolation resume_ -- --test-threads=1`: both new HTTP regressions failed with the expected missing-route 404; `/tmp/otto-review05-role1-rust-red.log`. Fixtures use migrated SQLite, the production sessions router and a disposable `/bin/cat` PTY; the live-process case cleans up before its baseline failure.

- **History:** one predicate includes working/running/idle in menu availability, detail action, labels and filters. Open in Chat calls `transcript.setView`, updating the reactive and persisted preferences. Every explicit open/resume uses the safe resume operation, including rows stale in either direction; a preflight GET followed by restart was rejected because it still races.
- **Safe resume boundary:** `POST /sessions/{id}/resume` uses existing owner/admin plus resource authorization and `ensure_live`, whose per-session lock and repeated live-handle checks preserve an active PTY. Existing late provider-ID discovery, Codex fork guard and provider/shell resume behavior remain. Archived and unsupported inactive sessions return Conflict; there is no destructive fallback. The workspace client updates returned session/status without incrementing restart nonce. Root owns shared HistoryStatus, API contract and policy integration.
- **Subagents:** explicit card consumers; last collapse/unmount aborts its request and removes its body immediately. One page (at most 60 turns) is retained per child, with opaque request-cursor metadata enabling earlier/newer navigation and reopening at the last page. A window-wide admission budget across parents limits retained bodies to 8 and conservative charge to 32 MiB, with 8 MiB/page. Oversized multi-turn pages retry internally with a smaller limit; one oversized turn has an explicit provider-transcript error and no impossible Retry. Capacity refusal retains existing visible bodies and tells the reader to close another card, then Retry. These are retained-payload estimates, not RSS or a bound on transient network/JSON parsing.
- **NewSession:** failed submitted requests survive mixed/all-failed outcomes; their captured prompt, cwd, title numbering, model/account/network/browser/extra-directory choices and scratch target are retried. Successful members leave the retry queue before navigation. Returning to the original workspace is required if a retained workspace-bound request would otherwise be redirected. The recovery surface stays open and reports remaining members; all-success closes normally.

## Edited paths

Production: `ui/src/modules/agents/history/HistoryPage.svelte`, `ui/src/lib/stores/workspace.svelte.ts` (safe resume method only), `crates/otto-sessions/src/http.rs`, `ui/src/lib/stores/transcript.svelte.ts`, `ui/src/modules/agents/conversation/SubagentCard.svelte`, `ui/src/modules/agents/NewSession.svelte`.

Regressions: `ui/unit/historyActions.test.ts`, `ui/unit/newSessionRecovery.test.ts`, `ui/unit/transcriptLifecycle.test.ts`, `crates/otto-sessions/tests/isolation.rs`, `ui/e2e/desktop-review4-session-recovery.spec.ts`. Existing direct child-load tests were adapted to acquire explicit consumers; parent teardown now clears bodies instead of retaining an empty loading/error shell.

## First checkpoint and adjacent regressions

Root executed the first implementation: **33/33 UI units passed** (`/tmp/otto-review05-role1-green1.log`), UI check **0 errors/0 warnings** (`/tmp/otto-review05-role1-check.log`), and both Rust resume route tests passed (`/tmp/otto-review05-role1-rust-green1.log`). These results precede the follow-up edits below.

Root review identified two adjacent boundaries. New tests reproduced both before their repair: **32 tests, 30 passing, 2 failing** (`/tmp/otto-review05-role1-followup-red.log`). A row previously marked live skipped resume after its process exited; 100 simultaneous child expansions started 100 response reads before retained-body admission. History now always uses safe resume. A child read now reserves its global count slot before the first fetch, releases it on failure/collapse, and guards against an old response releasing a reopened reader's reservation. Existing retained bytes remain charged during replacement reads. The deferred-response regression requires at most 8 requests before responses settle and verifies collapse aborts and frees capacity for a refused child. This bounds admitted readers; the 32 MiB accounting still describes retained payload estimates, not transient JSON buffers or RSS.

The new browser fixture uses a real isolated workspace/session and the actual mounted components. Its child scenario asserts exact cursor sequence, 60 rendered turns per page, oldest and newest reachability, and refetch at the preserved page after collapse. Its mixed-batch scenario injects one HTTP failure, edits the form after failure, verifies an identical failed request is retried, and checks server rows contain each successful member exactly once. These browser cases are authored but not yet executed. Only the two owned Rust files were formatted with `rustfmt --edition 2021`.

## Coverage checkpoint after independent recheck

Root subsequently executed the follow-up focused suite: **35/35 passed in 838 ms**. `iteration-4-role1-recheck.md` independently approved the inspected source direction without asserting final acceptance. This evidence predates the additional tests in this checkpoint; no production files changed here.

Four additional unit cases cover (1) a shared byte budget near 32 MiB across two mounted parents, crediting an old page during replacement, preserving a refused destination and retrying after another body closes; (2) adaptive 60→30→15-turn page reduction with all 180 turns reachable and navigation back to the newest; (3) late completion of a collapsed request after the same child reopens, protecting the replacement body and count reservation; and (4) a workspace-bound failed request A→B (no request dispatched)→A (identical original request succeeds).

The Rust route suite gains one inactive-success case using a custom fixture provider backed only by `/bin/cat`, migrated in-memory SQLite and the real router/manager. Concurrent safe opens and a subsequent open must return success and preserve the same live handle and recorded provider ID. Captured state is asserted after explicit PTY cleanup. Existing working/running/idle/stale-exited live-handle and authorization tests remain intact. The owned integration test file was formatted with edition 2021.

Three additional mounted browser cases cover all working/running/idle/exited/reconnectable row presentations against a live isolated shell (each open must use safe resume, preserve liveness and override a warmed Terminal preference in the same document); a stale working row after killing only its throwaway fixture shell (explicit open must make it live again); and on-disk import followed by a simulated resume conflict (stay on History, retry without a second import). The import response is deliberately stubbed to the existing shell identity; real provider transcript import/parsing is not claimed by that case. PTY identity preservation is asserted by the Rust router tests, while the browser verifies the mounted caller and resulting liveness.

Root's first additional-coverage run passed **38/39 UI cases in 674 ms**. The adaptive fixture alone failed because it assumed uniform 15-turn pages; near the beginning, the existing reduction rule uses the actual returned count and can reduce 60→29→14, leaving a subsequent 16-turn page. This is a fixture assumption, not a confirmed production defect. The test now checks a bounded nonempty count and serialized payload on each page, follows earlier cursors to completion with a finite guard, records the actual page count, and requires every exact ID 0–179 in both traversal directions. The initial full-page 60→30→15 assertion remains. No production change was made for this failure.

The corrected UI fixture and additional Rust/browser tests are **execution pending centrally**. The same focused commands below cover them: expected focused counts are 39 UI tests, 3 Rust `resume_` cases and 5 browser cases. No tests, builds, servers, commits, or production edits were performed in this coverage checkpoint.

## Central validation requested

```sh
cd ui
node --test unit/historyActions.test.ts unit/newSessionRecovery.test.ts unit/transcriptLifecycle.test.ts
npm run check
```

```sh
cargo test -p otto-sessions --test isolation resume_ -- --test-threads=1
```

```sh
cd ui
npx playwright test e2e/desktop-review4-session-recovery.spec.ts --project=desktop-browser --workers=1
```

Pending: additional coverage unit/Rust execution and latest typecheck; all five browser fixture cases; real provider transcript import remains covered elsewhere, not by the stubbed mounted import case; same-slot child-heavy CPU/RAM/latency measurement; further review waves and combined gates. No live daemon or user data was mutated by this role.
