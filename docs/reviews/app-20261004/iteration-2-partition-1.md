# Iteration 2 — partition 1 independent recheck

**Verdict: Approve with fixes — 0 blockers, 1 major, 1 minor remaining.** Original C1-01–04 defects are fixed by source trace. Two remaining Composer cases belong to the coordinated Claude UI effort and were sent to the root reviewer; no source fixes were made here.

Reviewed working tree at HEAD `431ed3fd`, including implementation wave 1 and integrated design PRs 75/76. Read the original C1/P1/UX1/D1 reports, `iteration-1-implementation-1.md`, `iteration-2-integration.md`, and `VERIFICATION.md`. Applied correctness and performance review methods. No builds, tests, servers, commits, or profiling were run by this reviewer. All independent evidence below is source hand-tracing; central test results are attributed separately.

## Ranked remaining findings

### R2-P1-01 — Major: reopening a composer bypasses its pending-image guard

**Original:** UX1-02, partially fixed. **Confidence:** confirmed by hand-trace.

**Locations:** `ui/src/modules/agents/conversation/Composer.svelte:60`, `:212`, `:275`–`:287`; keyed lifetime at `ui/src/modules/agents/conversation/ConversationView.svelte:878`–`:891`.

The new guard correctly blocks Enter and Send while the currently mounted composer's `uploading` counter is nonzero. However, that counter belongs to the component, while draft text, completed attachments, and the asynchronous upload's destination belong to the session store.

**Trace:** in session A, type “Fix this screenshot” and start an image upload whose response is delayed. `addImages` sets the old A component's `uploading=1` and awaits at line 282. Navigate to B and back to A before that response resolves. The keyed Composer is recreated; its `uploading` initializes to zero, while `transcript.draft(A)` still contains the text. Both `canSend` and the line-212 guard now allow submission. The request contains text with no image; success clears that draft. The old component's upload later resolves, appends its image to A's shared attachments, and decrements only its obsolete local counter. The screenshot again lands in the next draft instead of the intended prompt.

**Expected:** an in-progress upload belonging to A must keep A's send action gated across remounts and multiple views of the same session.

**Fix:** keep pending-upload count/state alongside session-owned drafts/attachments in `TranscriptStore`, update it in `addImages` with `try/finally`, and derive both button state and the authoritative `send()` guard from that shared state. Alternatively cancel uploads on unmount and retain an explicit recoverable pending attachment; do not silently forget the guard while allowing the original upload to finish.

**Regression:** delay an upload in A; navigate A → B → A; press Enter and click Send. Neither may submit until all A uploads settle. Resolve the upload, then submit once and assert text plus image path. Also test two panes showing A, and failure of one of several uploads. Existing same-component upload tests cannot cover this lifetime boundary.

### R2-P1-02 — Minor: slash popup minimum overrides the available-space clamp

**Original:** D1-02, partially fixed. **Confidence:** confirmed arithmetic trace; rendered trigger not executed here.

**Locations:** `ui/src/modules/agents/conversation/Composer.svelte:193`–`:195`, `:528`–`:539`.

The popup now measures its available room above the box, but then calculates `Math.max(72, Math.min(320, window.innerHeight * 0.5, room))`. The absolute popup still opens above the box, inside a clipping pane. With `room=30`, the computed maximum is **72**, not at most 30. A full suggestion list therefore extends beyond the pane's top. A short tile with attachment thumbnails under its box, or a sufficiently short resized split, can reduce this space below 72 px. The floor defeats the clamp exactly at its constrained boundary.

**Expected:** all scrollable popup content must remain inside its clipping region even when fewer than 72 px are available above the anchor.

**Fix:** honor the actual nonnegative room, including border/padding, or portal/reposition into a viewport-clamped surface when there is insufficient room. Do not force a minimum above the available budget.

**Regression:** seed 12 slash commands and a short chat pane with attachments, obtaining less than 72 px above the box; open `/`. Assert the complete popup bounds fit within the pane and viewport, then keyboard through the first and last options and verify each selected row is visible/hit-testable. Repeat after resizing while the menu is open.

## Original-finding disposition

“Fixed” below means the original failure is resolved by this independent source trace; it does not substitute for browser, provider, native-WebKit, or load-test sign-off.

| Finding | Status | Recheck evidence |
|---|---|---|
| C1-01: lookup errors prune existing transcripts | **Fixed** | `lifecycle.rs:55` distinguishes NotFound from other metadata errors; fallback scan remembers incomplete inspections and returns Unknown unless another candidate proves Exists. Both agy formats preserve Exists-over-error. Pruner retains Unknown. Deterministic ELOOP and manager retention regressions exist. |
| C1-02: disjoint reconnect loses reachable middle history | **Fixed** | `transcript.svelte.ts:238`–`:253` replaces a disjoint window with its fresh cursor/has_earlier. For old 1–60 and new 71–130, backward paging begins at 71 and reaches 61–70. An earlier request resolving after cursor replacement is ignored at line 275. Overlapping windows retain the original earlier boundary. |
| C1-03: async terminal paste crosses sessions | **Fixed** | `Terminal.svelte:747` captures session/socket/transform; post-upload checks reject a changed session, changed connection, read-only state, or closed socket. The successful write uses the captured socket. Reconnection to the same session intentionally requires a new paste if its socket changed. |
| C1-04: archive/restoration missing in other windows | **Fixed** | Manager emits `SessionArchiveChanged` after both committed transitions. `event.rs`, `ws_events.rs:369`, TypeScript union and WS contract include it. `workspace.svelte.ts:1542` applies one idempotent transition for HTTP/events, clears pending status, and updates active/archive membership. |
| P1-1: newline-free ring copies entire suffix per append | **Fixed** | `ring.rs:54` advances a logical head; compaction occurs after the dropped prefix reaches the ring budget, not every input chunk. Whole-line eviction subtracts only the logical retained prefix, and tail/search honor the offset. Copy cost is amortized O(input), with search copying the partial first line when needed. |
| P1-2: unbounded retained live folds / initial parser fan-out | **Fixed for original retained-state and initial-concurrency defects** | `transcript_tail.rs` now caps source size, record count, per-tail charge and aggregate reservations; RAII releases charge. Initial folds share two permits with the offline cache. Failed initial/incremental admission retires the folder and retains a coalesced metadata watcher. See resource-admission caveat below; no RSS claim. |
| P1-3: one live page clones all history | **Fixed** | `page_of` calls `Folder::page`; `Fold::window` clones only selected finalized turns (plus the candidate at the byte boundary). Full tool history no longer clones for a tool expansion. Cumulative metadata/stats and pending notes/blocks remain represented. Fixture-prefix equivalence tests exist. |
| P1-4: history hold never released on return to live | **Fixed** | `followLive` clears hold and trims using recoverable bounds. Jump to latest, reaching the end through Load later, and scrolling to the live bottom call it. Subsequent deltas resume trimming; count and payload budgets complement each other. Explicit historical reading and the newest oversized single-turn case remain exceptions. |
| P1-5: fallback filesystem resolution blocks runtime | **Fixed** | `routes/transcript.rs:188` clones resolver inputs into the blocking helper, then persists the resolved path asynchronously. |
| UX1-01: session-list failure masquerades as empty/foreign workspace | **Fixed for original selected-workspace scenario** | Selection clears foreign rows and tabs, records generation-scoped errors, and withholds old panes while layout is unready. Agents renders Retry before empty onboarding. Retry repeats discovery and completes saved-layout restore. Stale request failure cannot overwrite a newer selection. |
| UX1-02: send during image upload | **Not fully fixed** | Same mounted composer is guarded; session remount/multiple-view ownership remains broken as R2-P1-01. |
| UX1-03: palette fetch failure looks like no results | **Fixed** | Palette retains a visible query-owned error/Retry; controller identity and abort checks protect both success and failure. Debounced search captures the workspace, and search effect tracks workspace reads through `scheduleSearch`. Local commands remain available. |
| D1-01: long draft overflows short tile | **Uncertain — source repair present, rendered acceptance not established** | Autosize uses half the measured parent height (and window cap), a ResizeObserver recalculates it, and composer/box minima allow shrinking. This removes the original purely window-sized 360 px textarea in a 220 px tile. No rendered long-draft/attachment/control-hit-test evidence was supplied for the smallest tiles, so usable conversation height and reachable controls are not independently signed off. |
| D1-02: slash popup clipped above pane | **Not fully fixed** | Measurement added, but 72 px minimum defeats smaller budgets; R2-P1-02. |
| D1-03: covered chat retains keyboard focus under preview | **Fixed by source trace; native focus verification pending** | `PreviewPanel.svelte:93` observes covering layout. Lines 106–118 make covered siblings inert, move focus into preview, and restore previous focus/inert state on close or becoming side-by-side. Parent's synchronous list-focus attempt is inert while covering; cleanup then restores the trigger. |

## Failure, cancellation, and resource checks

- **Transcript file growth:** metadata is read before admission; `Tailer::poll_up_to` consumes only that admitted prefix, so appended bytes remain for the next poll. Its byte/record-limit failures occur before advancing the cursor. Crossing source/charge limits retires state and emits an explicit empty delta; later fallback metadata changes invalidate at most once per 15 seconds. Offline folds stream a fixed prefix and reject excessive input/record/charge with an explicit error, leaving source files untouched.
- **Reservation boundary caveat:** the first incremental reservation at `transcript_tail.rs:496` includes incoming source bytes but only the *existing* record count. The additional `2048 × new_records` charge is reserved after `Tailer` has constructed its parsed delta (`:531`–`:540`). Thus it bounds retained folds before applying records, but does **not** establish pre-parse admission for every transient incremental allocation. Incremental polls also do not take the two initial-fold permits. These temporaries have explicit source/record bounds; without measurements I do not assert a new scalability failure. Root's load pass should include simultaneous record-heavy bursts against nearly full aggregate admission, rather than equating reserved charge with peak RSS.
- **Refold failure:** a replaced file or Codex-era transition that cannot reserve a fresh folder retires to fallback rather than continuing to publish an incomplete fold. Old budget remains held while a replacement is constructed, so the policy may conservatively fall back before steady-state capacity would require it; no incorrect result was established from that choice.
- **Read cancellation and paging:** tail reads reject abort/inactive completion; earlier reads reject changed reader epochs and changed paging cursors. Recovery uses fresh metadata while ordinary earlier-page completion preserves newer transcript metadata instead of overwriting it with its pre-await snapshot.
- **Terminal delivery:** a replacement socket, a different session, and read-only transition reject delayed paste with an actionable toast. This is intentionally different from the Composer upload lifetime hole above.
- **Session discovery and search:** stale generations/controller completions cannot convert a newer successful context into an error. Session failures remain retryable without requiring a socket reconnect. Archive-state events use the same owner-scoped WS classification as session creation.

## Verification attribution and remaining limits

Root reports the integrated UI suite at **1,062 passing**, full PTY/transcript tests passing, focused session Rust tests passing, real-wheel History paging E2E passing, and API guard browser flows passing. Earlier per-finding red/green evidence and known invalid harness runs are recorded in `VERIFICATION.md`; this reviewer did not rerun or independently observe those executions. Reviewed representative added tests for disjoint paging, follow-live bounds, terminal async ownership, streaming-prefix handling, admission rejection, and bounded-folder equivalence.

Final CPU/RAM measurements under concurrent agents, memory recovery over time, and real-application sampling are **pending at the root**. No performance measurement, production capacity guarantee, or memory-leak clearance is claimed here. No standalone rendered validation of the remaining slash boundary, long composer layouts, remounted uploads, or native focus/VoiceOver was performed. Only this report was written.
