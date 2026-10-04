# Iteration 1 — user experience, partition 1

**Verdict: fixes needed — 2 major, 1 minor.** Baseline `a16f4c71`. Scope: shell/navigation, agent sessions, terminal and conversation workflows. This is a finite review of current behavior, not a feature wishlist.

Read `AGENTS.md`, the design guidelines (`README`, `patterns`, `content`, `components`), the coordination file, and partition 1's correctness/performance reports. All findings below are **source-traced**, not browser reproductions. No builds, servers, tests, source edits, commits, or additional agents were run. Only this report was written.

## UX1-01 — Major: failed session discovery looks like an empty or still-valid workspace

**Scenario:** An authenticated user opens Agents in a workspace that has existing sessions, but its session-list request fails while the rest of the daemon remains reachable. Alternatively, they switch from workspace A to B and B's request fails.

**Locations:** `ui/src/modules/agents/AgentsPage.svelte:55` and `:63`; `ui/src/lib/stores/workspace.svelte.ts:538`, `:831`, `:888`; callers `ui/src/shell/App.svelte:342` and `ui/src/shell/Navigator.svelte:956`.

**Evidence and impact:** The store records loading but no session-load error. It rethrows the current request's failure and clears `sessionsLoading` in `finally`. The shell's `void ws.load()` and sidebar's `ws.select(w.id)` do not provide a local failure view. Agents then chooses among the coach, “No sessions yet,” existing panes, tiled view, or mission view without an error branch. On an initially empty store, existing work can therefore appear absent and the user is offered creation/onboarding rather than recovery. On a failed workspace switch, `select` has already changed `currentId` but the old `sessions` and layout remain because replacement/restoration never completes. The user cannot reliably tell which workspace's work is displayed. A socket reconnect can eventually refresh sessions, but a single failed HTTP request does not require a socket reconnect, and no local Retry is offered.

**Smallest fix:** Retain a generation- and workspace-scoped session-load error and render a recoverable error region in Agents before any genuine-empty onboarding decision. Preserve same-workspace stale data with a clear refresh failure; do not present another workspace's old list/panes as B's. Retry the failed selection/load and finish layout restoration on success. Do not implement Retry solely as `select(currentId)`: the existing early return at line 539 accepts a nonempty stale list. Keep failed loads from consuming the one-time auto-open decision (`AgentsPage.svelte:38`).

**Verification:** In an isolated browser test, allow auth/workspace listing but fail the selected workspace's sessions GET. Verify a persistent error and Retry, no “No sessions yet”/coach, then restore the endpoint and retry to the existing session. Repeat A → B with A's list already loaded; verify A's sessions are not represented as B's and successful retry restores B's layout. Cover split, tiled, and mission modes. This session-specific recovery is explicitly available to Codex in the coordination file; it is separate from Claude's Home error work.

## UX1-02 — Major: Send accepts the prompt while its intended image is still uploading

**Scenario:** A user types “Fix the problem shown in this screenshot,” pastes or attaches the screenshot, and immediately presses Enter or clicks Send while the image request is still in progress.

**Locations:** `ui/src/modules/agents/conversation/Composer.svelte:173`, `:227`, `:239`, `:273`, `:331`, `:353`.

**Evidence and impact:** `addImages` increments `uploading`, then awaits `uploadInboxImage`; the attachment enters the draft only after that await. `canSend` checks only `sending` and existing text/attachments. The keyboard handler calls `send()` directly, and `send()` also has no upload guard. Consequently the text is submitted without the screenshot, the submitted text clears, and the finished upload later appears in the next draft. The small upload-status line acknowledges progress but the primary action remains available with its ordinary Send behavior. The agent can start the requested work without the context the user just attached; the user must notice and send a corrective message. With multiple files, a partial subset can be submitted.

**Smallest fix:** Prevent both mouse and keyboard submission while any selected images are uploading, and explain that the images are still uploading near Send. The guard must live in `send()` as well as the button's disabled state. Keep text and successful attachments intact when an upload fails so the user can retry or deliberately send the remaining draft. Preserve existing session ownership across navigation.

**Verification:** Defer an image-upload response, type a prompt, select an image, then exercise Enter and the Send button separately. Neither should call `submitPrompt` before the image is ready. Resolve the upload; one submission must contain both the text and image path. Repeat with two images where only one has completed and with one failed upload; ensure no partial submission occurs during the pending interval and the draft remains recoverable. This differs from correctness finding C1-03: that concerns delayed terminal image paste targeting a different session, while this concerns incomplete context submitted from the chat composer.

## UX1-03 — Minor: workspace search failure is indistinguishable from no matching content

**Scenario:** A user searches in ⌘K for a known workspace item while the workspace search endpoint returns an error, with local commands still available.

**Locations:** `ui/src/shell/Palette.svelte:80`, `:89`, `:509`.

**Evidence and impact:** Non-abort errors explicitly clear `searchHits`; `finally` clears the busy state. The results section is shown only while busy or when hits exist. A failed remote search therefore removes “Searching…” and leaves local commands/Ask Otto with no indication that content search failed and no retry action. The user can conclude the item is missing, change a correct query unnecessarily, or abandon the navigation path. Local command availability is useful, but does not explain the missing remote results.

**Smallest fix:** Track a query-scoped remote-search error separately from hits. Keep commands usable, show the failure within the results region, and provide Retry for that same query. Clear the error on query/workspace changes; aborted superseded searches should remain silent. Coordinate the small markup addition with Claude's ownership of palette/overlay presentation; the finding is about fetch-state/recovery behavior, not overlay styling.

**Verification:** Fail the search endpoint for a nontrivial query while retaining command results. Verify that commands still work and the search error remains visible. Retry after restoring the endpoint and verify the expected result can be opened without retyping. Separately verify that a successful empty result is not presented as an error, and aborting an old query does not show a failure for the new query.

## Observed screenshot and confirmed useful paths

Viewed `/tmp/otto-review-load-baseline-fixed/shot-n1-tiled.png` with the image tool: one light desktop view shows a selected session in the navigator, its title/provider in the pane header, Terminal/Chat controls, active terminal output, and an ambient working count. This supports only the normal layout's visible affordances; it does not demonstrate interaction, recovery, upload timing, keyboard behavior, or a failure state. None of the three findings is claimed as visually reproduced.

Source tracing also found existing useful recovery: terminal reconnect/resume controls (`Terminal.svelte:2914`–`:2936`), a chat approval card that links to the terminal (`LiveStatus.svelte:74`), explicit archive-list retry (`Navigator.svelte:1457`), and History's failed-refresh retry (`HistoryPage.svelte:395`). The chat composer preserves the draft after a failed submit. These paths were not exercised dynamically.

## Coverage limits and next wave

Read/traced selected paths in AgentsPage, workspace selection/loading, navigator search/archive, palette remote search, NewSession's batch outcome, SessionView actions, tiled-view bookkeeping, terminal connection overlays, conversation loading/search/tail controls, composer submission/uploads, and History recovery. This was not an exhaustive audit of every interaction in these large files.

Did not repeat partition 1 findings C1-01–04 or P1-1–5. RTL, shared primitives, overlay geometry, Lightbox/tablist/label accessibility, Home errors, trust confirmations, and general copy remain with the coordinated design effort. No independent UI-style recommendations are included.

Next wave should run the three targeted scenarios above on an isolated daemon/browser fixture, then cover keyboard task completion through workspace switching, split/tiled session selection, and Chat → terminal approval → Chat. Native macOS focus, actual provider queues, slow network behavior, narrow layouts, dark mode, cross-window recovery, and assistive technology were not validated here. The normal screenshot and source checks do not establish those behaviors.
