# Iteration 1 — correctness, partition 1

**Verdict: Block — 1 blocker, 3 major; all four confirmed by source hand-trace.**

Baseline: `a16f4c71`. Scope: current application, not a diff. Read-only investigation; only this report was written. No builds, application launches, automated tests, or real-data probes were run.

## C1-01 — Filesystem lookup errors cause permanent deletion of resumable background sessions

- **Severity:** blocker. **Confidence:** confirmed by hand-trace.
- **Locations:** `crates/otto-sessions/src/lifecycle.rs:64`, `:72`, `:82`; destructive caller `crates/otto-sessions/src/manager.rs:4950`–`:4954` and `:5103`.
- **Expected:** the documented pruning rule is to delete a session only when its transcript is positively absent; inability to inspect it must return `Resumability::Unknown` and retain the row.
- **Concrete trace:** an exited background Claude session has a valid transcript, but the daemon cannot search its project directory (e.g. the `.claude/projects` directory temporarily lacks search permission). `Path::is_file()` returns false on the metadata error. `read_dir` either fails or its candidate `is_file()` checks also fail. The function unconditionally returns `Gone` at line 82. `prune_dead_sessions_with_home` passes the foreground/account/live guards for this background session, matches `Gone`, and calls `remove`, deleting the SQLite session row and retiring its associated credentials. The transcript still exists; Otto nevertheless loses the session. The periodic caller is `crates/ottod/src/main.rs:988`.
- **Fix:** use fallible metadata/read-directory operations. Return `Gone` only after a complete absence check; map permission, I/O, and incomplete iteration errors to `Unknown`. Preserve the existing exact-path `Exists` fast path. Apply the same fallible treatment to `agy_conversation_exists`, whose `is_file()` calls have the same ambiguity.
- **Regression:** create an actual transcript for a background session, make its directory unsearchable (on a non-root test user), and assert the helper returns `Unknown` and pruning retains the row. Alternatively inject a filesystem reader that returns `PermissionDenied` for metadata/read_dir. Also retain positive-absence and successful-fallback tests.

## C1-02 — A recovery gap longer than one transcript page becomes unreachable history

- **Severity:** major. **Confidence:** confirmed by hand-trace.
- **Locations:** `ui/src/lib/stores/transcript.svelte.ts:211`–`:225`, `:239`–`:251`; server cursor semantics `crates/otto-transcript/src/fold.rs:76`–`:110`.
- **Expected:** reconnecting an open conversation should recover missed turns, or expose an accurate paging boundary from which every missing turn can be loaded.
- **Concrete trace:** initially load turns 1–60, with the server reporting `has_earlier=false`. Disconnect the events socket while the agent appends turns 61–130. Reconnection invokes `transcript.resyncVisible` (`ui/src/lib/events.svelte.ts:532`), which requests only the newest 60 turns, 71–130. `readTail` merges those with existing 1–60, producing `[1…60, 71…130]`, and explicitly retains the old `has_earlier=false` and cursor at line 222. Turns 61–70 never arrive, and `loadEarlier` immediately returns at line 241. If the old page already had earlier history, its old cursor similarly pages before the old head, never into the middle gap. A reload can recover these turns, but ordinary reconnect and paging cannot.
- **Fix:** do not concatenate disjoint windows while claiming a single continuous cursor. On no overlap, either backfill using the fresh page's cursor until the old tail is reached, or replace the displayed window with the fresh page and its `cursor`/`has_earlier` (and preserve user viewport intent explicitly). Keep the current merge only when continuity is established.
- **Regression:** seed a 60-turn conversation with no earlier page; simulate a disconnected interval containing 70 new turns; resync. Assert all 130 turns are present or that the newest page exposes a working earlier cursor that retrieves every missing turn. Repeat when the original window already had `has_earlier=true`.

## C1-03 — Delayed image paste goes into the newly selected terminal

- **Severity:** major. **Confidence:** confirmed by hand-trace.
- **Locations:** `ui/src/lib/components/Terminal.svelte:745`–`:753`, `:640`; component reuse `ui/src/modules/agents/SplitNode.svelte:219`, `ui/src/modules/agents/SessionView.svelte:962`; session retargeting `ui/src/lib/components/Terminal.svelte:2570`–`:2631`.
- **Expected:** an image pasted into session A must remain owned by A when the user switches tabs while uploading.
- **Concrete trace:** paste an image into A. `uploadPastedImage` awaits PNG conversion and the HTTP upload. Switch the same split pane to B before the upload returns. The unkeyed `SessionView`/`Terminal` is reused, and the session-switch effect changes `sock` to B (with `keepAlive`, `switchEngine` adopts/builds B). When the upload resolves, line 753 calls `sendJson`, which reads the *current* `sock` at line 642. If B is connected, B receives A's image path as bracketed input; A receives nothing. A changed `readOnly` state is also not rechecked after the await.
- **Fix:** capture the destination session and socket/generation before awaiting. Deliver only to that original authorized destination; if it is no longer available, retain a session-owned pending attachment/input or report that the paste could not be delivered. Never send through a mutable current-session socket after an async upload.
- **Regression:** defer the image-upload response, paste in A, switch the pane to B, then resolve the upload. Assert B receives no input and the original paste is delivered to A or remains explicitly pending for A. Cover disconnect/unmount during upload too.

## C1-04 — Archive/unarchive changes do not synchronize to other windows

- **Severity:** major. **Confidence:** confirmed by hand-trace.
- **Locations:** `crates/otto-sessions/src/manager.rs:5022`–`:5028`, `:5090`–`:5099`; consumer `ui/src/lib/stores/workspace.svelte.ts:1631`–`:1689`.
- **Expected:** all windows subscribed to the session's workspace should reflect archive membership and restoration, as they already do for creation and deletion.
- **Concrete trace:** two connected windows display foreground session A. Window 1 archives A. The manager persists `archived=true` but emits only `SessionStatus::Exited`; window 1 moves the row using the HTTP response. Window 2's event handler updates only status and last-active time; foreground A remains in its active session list with `archived=false`, and its archive list is not invalidated. The server comment that clients refresh on this event is incorrect: the handler does not refresh. Attempting Resume from window 2 now fails because the server knows A is archived. Unarchiving in window 1 is worse: `unarchive` emits no session event at all, so window 2 cannot discover the restored row. `record_lifecycle` emits activity trail information, not an archive-membership session update. State remains stale until an unrelated refresh/workspace switch/reconnect.
- **Fix:** publish a session archive-state update carrying the authoritative row, or a dedicated archive/unarchive event that causes scoped list refresh and archived-page invalidation. Implement both transitions in the UI store and update the Rust/TypeScript/WS contracts together. Avoid treating every ordinary `Exited` status as an archive event.
- **Regression:** use two independent workspace stores fed the same daemon event stream. Archive and then unarchive via one client, without reconnecting either. Assert both clients have identical active/archive membership, and the restored row can be reopened from either window.

## Reviewed surfaces and limits

Inspected shell boot and route preparation/history/leave guards (`ui/src/App.svelte`, `ui/src/lib/router.svelte.ts`, selected shell integration), workspace selection/list refresh and session mutations/event application, split session-view reuse, conversation composer ownership and send flow, transcript lifecycle/recovery/paging, terminal connection/snapshot/exit/restart/input paths, session REST routing, and manager lifecycle/resume/archive/prune/input authorization paths. Traced callers into server transcript pagination, events dispatch, and periodic daemon lifecycle jobs.

Existing protections verified while tracing include keyed composer ownership across navigation; create-session insertion guarded by current workspace ownership; reconnect refresh of workspace lists; restart/ensure-live serialization; archived-session resume refusal; and captured process generation for delayed human-submit Enter. No additional finding is asserted for these paths.

Coverage is source-based and selective within this large partition. Did not execute Svelte effects, browser/PTY integration, native macOS shell behavior, transcript filesystem error tests, provider binaries, or exhaustive WS transport branches. Did not audit unrelated product modules, provider protocol details, terminal rendering performance, or security beyond correctness of the traced state transitions. No claim that the remaining application is defect-free.
