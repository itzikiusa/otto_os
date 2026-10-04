# Iteration 1 implementation — session runtime / role 1

Status: implementation ready; focused UI regressions passed centrally. Rust checks and final integration checks are pending with the root build owner. No builds, test suites, servers, commits, merges, or live-daemon mutations were executed by this agent.

## Findings and repairs

All assigned findings were confirmed by source tracing; no assigned finding was rejected. UX1-02 (Composer uploads) belongs to the coordinated Claude effort and was not edited here.

- **C1-01:** fallible transcript metadata and complete fallback-directory inspection distinguish `Unknown` from positively `Gone`. A present alternate provider format still wins over another path's error. Deterministic symlink-loop regression reproduced `Gone` before the repair, including when tests run as root; a manager regression verifies a background session's row survives the lookup failure.
- **C1-02:** reconnect merges only overlapping windows. Disjoint recovery adopts the fresh page and its cursor/`has_earlier`, so every missed turn remains reachable. A historical request that resolves after its paging cursor was replaced is ignored. Tests cover original `has_earlier` both true and false.
- **C1-03:** terminal image paste captures session, socket and frame transformer before conversion/upload. It rechecks ownership, writable state and connection before sending to the captured socket; failed delivery reports an actionable error. Deferred-upload tests reproduced wrong-session delivery, disconnected delivery, and changed read-only delivery before the repair; the unchanged destination case remains successful.
- **C1-04:** archive and restoration emit `session_archive_changed` with the committed authoritative session row. Rust event naming, WS session-owner scope, TypeScript union and WS contract agree. HTTP responses and events share one idempotent store transition for active/archive membership and status. Tests use two independent window stores and two manager event subscribers.
- **P1-1:** newline-free ring overflow advances a logical prefix instead of copying the retained suffix on every append. Compaction is amortized; search copies only an offset first line at read time, preserving snapshots. An isolated counting-allocator regression measured **201,326,592 allocated bytes for 1 MiB output** on the old implementation (64 KiB retained ring), against an 8 MiB budget. An earlier pointer-address test was rejected because allocator address reuse made it pass the old implementation; it was removed.
- **P1-2:** live folds enforce source/record limits and conservative per-tail/aggregate reservations, including sidecar metadata. Reservations precede folding and incremental source parsing. Initial live folds and the offline cache share two worker permits. Oversized/unadmitted folds keep a small file-change watcher with at most one recovery invalidation per 15 seconds. Offline reads stream a fixed file prefix, with a two-pass Codex prescan, and explicitly reject resource limits rather than allocating the whole input/JSON-record vector or silently truncating history.
- **P1-3 / P5-01 API handoff:** `Folder::page` finalizes/clones the selected page only; `Folder::bounded_snapshot` clones only the bounded newest turns; direct `tool_block` lookup avoids a full clone when expanding a result. Cumulative stats, pending notes/blocks and subagent placement are preserved. Fixture-prefix equivalence tests compare the bounded API with the original full snapshot for Claude and both Codex eras.
- **P1-4:** returning to live following releases the history hold and trims at known recoverable cursor boundaries. Live windows have both a 600-turn cap and an 8 MiB estimated UTF-16 payload cap, with a 6 MiB target. Turn charges are cached by immutable object identity. “Jump to latest”, reaching the live end through “Load later”, and scrolling to the live bottom release the hold. Deliberately viewed historical pages remain while the user reads them.
- **P1-5:** filesystem transcript fallback resolution runs in the existing blocking helper, preserving async persistence of the resolved path.
- **UX1-01:** session discovery records a selection/generation-scoped error; failed workspace switching clears foreign rows and does not restore/render the previous layout under the new workspace. Retry completes the interrupted selection/layout restore. Agents shows inline recovery before genuine-empty onboarding and does not consume its auto-open decision on failure. Same-workspace stale content can remain with an explicit refresh error.
- **UX1-03:** palette search has query-owned error/retry state. Superseded or aborted responses cannot replace newer results or show stale failures; local command search remains available.

## Verification supplied to root

Central root execution confirmed these red stages before their fixes:

- `cargo test -p otto-sessions --lib lifecycle::tests::unreadable_transcript_is_unknown_not_gone` — `Gone` versus required `Unknown`.
- `node --test unit/transcriptLifecycle.test.ts` — two disjoint recovery cursor failures; subsequently history-hold reset missing; subsequently byte-budget overflow.
- `node --test unit/asyncOwnership.test.ts` — foreign-workspace stale rows and absent archive-event membership update.
- `node --test unit/sessionPasteOwnership.test.ts` — three invalid destinations sent input; happy destination passed. An initial harness selected the module script rather than instance script; corrected before valid red evidence.
- `node --test unit/paletteSearchRecovery.test.ts` — hidden error and stale failure wiping current hits.
- `cargo test -p otto-pty --test ring_allocation` — allocation volume exceeded the budget (201,326,592 bytes).

Root subsequently reported **40/40 focused UI tests passed**, covering transcript lifecycle/stability/byte bounds, session ownership/recovery, terminal paste, and palette recovery (`/tmp/otto-review-session-ui-green2.log`). Initial broader UI check found only role-2 ClickHouse typing failures, which that owner was repairing; final merged UI checks remain necessary.

Focused Rust commands (run centrally):

```bash
cargo test -p otto-pty --test ring_allocation
cargo test -p otto-pty --lib ring::tests
cargo test -p otto-transcript --test fixtures
cargo test -p otto-transcript --lib
cargo test -p otto-sessions --lib lifecycle::tests
cargo test -p otto-sessions --lib archive_and_restore_emit_authoritative_rows_to_every_subscriber
cargo test -p otto-sessions --lib prune_keeps_background_session_when_transcript_lookup_fails
cargo test -p otto-server --lib transcript_tail::tests
cargo test -p otto-server --lib transcript_cache::tests
```

New Rust regression coverage includes source/record/charge rejection, fixture streaming equivalence, bounded-page equivalence after each fixture record, tool lookup presence, sidecar size rejection, admitted-prefix reads preserving concurrent appends, aggregate reservation saturation/rollback, initial oversized input refusal, and incremental fallback invalidation. Existing tail stepping and page-settlement tests remain relevant. Root should run the scoped repository gate, E2E recovery/visual checks, and CPU/RAM workload measurements after integration.

## Bounded Folder API for role 5

- `Folder::bounded_snapshot(limit: usize, byte_limit: usize) -> Folded`: newest finalized turns, metadata and cumulative stats; does not clone old turns, provider indexes, or the artifact registry. `artifacts` in the returned snapshot is empty; call `Folder::artifacts()` separately if needed. Like the existing page contract, one newest eligible turn is kept even if it alone exceeds `byte_limit`.
- `Folder::page(before: Option<usize>, limit: usize, subagents: Vec<SubagentMeta>) -> Transcript`: equivalent to full-snapshot paging, including `has_earlier`, while copying only the selected turn window.
- `Folder::tool_block(tool_id: &str) -> Option<Block>`: clones one indexed tool block.
- Existing `record_count()` and `turns_since(since)` provide role 5's incremental reply indexing. Bounded accessors still scan turn descriptors / placement metadata; this is not a claim of fully O(delta) CPU.

## Limits and outstanding verification

Live source limits are 8 MiB and 16,384 records; charge ceilings are 32 MiB per tail and 128 MiB aggregate. Charge is conservative source-byte/record/sidecar accounting, **not measured RSS**. Sidecars have an 8 MiB aggregate charge and 1 MiB individual source cap. Offline reads cap input at 128 MiB, one record at 16 MiB, and cumulative fold charge at 256 MiB; two worker slots constrain parse concurrency. Limits are documented in the feature guide and HTTP contract. Exceptional files return HTTP 413 with a resource-limit explanation; data remains on disk and can be opened in the provider or read through a smaller subagent transcript.

Historical windows intentionally held while reading are not trimmed until live following resumes. A reconnect with a disjoint new page changes the visible window to the newest page while retaining correct backward recovery. Native WebKit viewport behavior, light/dark/mobile rendering, multi-window real transport, aggregate CPU/RAM and full consumer compilation remain root verification responsibilities; no results are claimed for them here.

## Adjacent History keyboard test diagnosis

The existing `desktop-ux-r4-insights.spec.ts` History test focused the “Load earlier messages” button and expected focus alone to paginate. The first-mount scroll-settle guard can return the view to the bottom before a focus-induced scroll triggers automatic pagination. The button already supports native Enter activation. This is a test expectation error: focus then `page.keyboard.press('Enter')` exercises the intended keyboard workflow without relying on browser scrolling. Root forwarded that narrow correction to Claude, who owns the test; no competing test/UI edit was made here.

A separate browser regression, `ui/e2e/desktop-history-pagination.spec.ts`, performs real wheel scrolling after the one-second initial layout guard and asserts the exact `120` then `60` earlier cursors and final oldest turn. It does not depend on focus causing scroll. Run centrally after main integration:

```bash
cd ui
OTTO_E2E_SLOT=review04 OTTO_E2E_PORT=7814 OTTO_E2E_PW_PORT=5314 OTTO_E2E_SWEEP_ORPHANS=0 npx playwright test --project=desktop-browser e2e/desktop-history-pagination.spec.ts e2e/desktop-ux-r4-insights.spec.ts --workers=1
```

Final source-readiness checks: `rustfmt --edition 2021` completed successfully on the assigned changed Rust files; `git diff --check` passed for the assigned Rust/contract paths. These are formatting checks, not compilation or execution evidence. The ring allocation test now prints exact successful allocation volume with `--nocapture` for root's before/after record.


## Final terminal-link fixture investigation

Read Claude's failed `e2e8.log` / `e2e9.log` and the retained Playwright trace archives under `/private/tmp/claude-501/-Users-itziklavon-claude-ade/fb25ab39-e879-46a9-ba8b-0aae6789df56/scratchpad/`. All four failing link cases recorded `mouseMove` and `mouseClick` at **x=0, y=0**:

| Run / case | Trace input call IDs (move / click) |
|---|---|
| pw8 relative reference + OSC8 | `call@866` / `call@870` |
| pw9 wrapped path + HTTP URL | `call@276` / `call@280` |
| pw9 Codex cyan CRLF, wide | `call@132` / `call@136` |
| pw9 Claude bold cursor continuation | `call@105` / `call@109` |

These traces prove the recorded failures did not click their requested terminal text. They do not establish a production link-parser defect. The global fixture setup explicitly forces the DOM renderer through `otto.term.renderer=dom`; `.xterm-rows` is intentional here. xterm's DOM renderer replaces row children on repaint, while the old helper checked visibility before separately obtaining/evaluating a handle. A detached text range between those operations is a plausible cause of its zero rectangle, but the existing traces do not record `isConnected` to prove that exact interval.

Changed only `clickText` in `ui/e2e/desktop-terminal-links.spec.ts`: poll for the requested substring's connected text range with finite, positive dimensions and a center inside the viewport before moving/clicking. Retained the original 80ms hover interval after geometry polling to keep the repair scoped; that delay is not the geometry-readiness condition. The helper performs one click and retains the existing product assertions, including negative link cases. Production Terminal and link parsing are unchanged. No builds, servers or tests were run by this role; root owns the focused browser rerun after load measurements:

```sh
cd ui
OTTO_E2E_SLOT=review04 OTTO_E2E_PORT=7814 OTTO_E2E_PW_PORT=5314 OTTO_E2E_SWEEP_ORPHANS=0 npx playwright test --project=desktop-browser e2e/desktop-terminal-links.spec.ts --workers=1
```

Source follow-up: xterm Linkifier handles mousemove → provideLinks → currentLink synchronously for the two installed providers (native OSC8 and Otto text links); neither provider defers its callback through a timer, Promise or animation frame. Mouse-down captures the current link and mouse-up checks that same target. Renderer viewport changes can invalidate/recompute hover independently. No mandatory 80ms linkifier delay was established from this source, but the original delay is preserved to avoid changing hover timing in the geometry-only repair.


### Follow-up: external URL assertion observed the obsolete opening mechanism

Root's geometry-repair repeat run reported 27 passing / 3 failing cases (`/tmp/otto-review-terminal-repeat.log`). All three failures were the wrapped-path/HTTP test's final URL assertion; its wrapped file assertion now passed. All three retained traces under `/tmp/otto-review-terminal-repeat/desktop-terminal-links-wra-01a7d-URL-clicks-use-full-targets-desktop-browser*` show a new BrowserContext page with the originating page's `openerPageId` immediately after the HTTP-link click.

Production `Terminal.activateLink` calls `openExternal`; its web path in `ui/src/lib/external.ts` uses `window.open(href, '_blank', 'noopener,noreferrer')`. The old test listened only for an anchor click to populate a body attribute, so it could not observe that successful opening route. Replaced only that fixture observer/assertion: context-route `https://example.invalid/**` to a local response, register a popup listener before clicking, and assert the actual popup URL equals `https://example.invalid/report?q=yes`, then close it. The exact path and query assertion remain; no production URL routing changed. Central rerun is pending, and this role ran no browser/build/test.


### Post-main History scroll fixture startup ownership

Root's combined browser suite passed 97 cases; the only failure was `desktop-history-pagination.spec.ts` expecting one initial request. The failure received `[null, null, null, "120"]` rather than `[null, "120"]`. Trace `/tmp/otto-review-merged-browser/desktop-history-pagination-100b0-fter-initial-layout-settles-desktop-browser/trace.zip` records the scratch transcript read at monotonic time 92201.428, workspace-restored reads at 92205.739 and 92232.560, then the real wheel at 93605.210 and `before=120` read at 93658.339. No HMR reload appears in that trace. The wildcard fixture supplies the same row across startup scopes; merged History automatic selection can read it before workspace restoration settles.

Updated only the browser test: after its existing initial settle, require a nonempty set of initial requests whose cursors are all null, then capture that startup count. The two actual wheel actions must produce exactly `["120"]` followed by `["120", "60"]` after that baseline. Duplicate pages and renewed null-cursor reads during scrolling still fail. The oldest-turn and absent-earlier-button assertions remain. This change does not replace scrolling with button activation or change production pagination. Root's three-repeat combined History/Terminal rerun is pending; this role ran no tests/builds/servers.


### Central closing execution

Root's merged History + terminal suite passed all 33 invocations (11 cases repeated three times) in 28.5 seconds. Both real scroll cursors and the exact external popup URL are verified; no production changes were required for these fixture repairs. Log: `/tmp/otto-review-paging-links-repeat.log`.
