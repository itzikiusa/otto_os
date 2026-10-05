# Iteration 4 — UX, partition 1

Source: `03f2bc3e380cbee6da22723872ba865a84aa8050`, branch `fix/app-review-20261005`. Reviewed shell/navigation, session creation, session/terminal/conversation recovery, handover and History. **Provisional score: 7.1/10; not accepted.** One new minor recovery finding; one additional minor search-scope finding forwarded for Claude ownership. Existing correctness blocker R4-C1-01 independently prevents acceptance.

This was a bounded source review. Read `AGENTS.md`, `PLAN.md` (including execution calibration), design guidelines README/patterns/content, current coordination and Claude's consolidated findings. No builds, tests, servers, rendered inspections or source edits were performed. Only this report was written. User steps below are source-traced scenarios, not executed reproductions.

## New finding

### R4-U1-01 — Minor — partial batch creation discards the failed sessions' configuration

**Location:** `ui/src/modules/agents/NewSession.svelte:371`–`:378`; state initialized at `:72`–`:85`; persisted preferences at `:366`; actual unmount at `ui/src/shell/App.svelte:1288`–`:1295`. Error toasts expire after 12 seconds (`ui/src/lib/toast.svelte.ts:78`–`:82`).

**Steps:** Open New session; request two agents; supply an opening prompt, custom title/folder and optional account/additional folders. Allow the first create request to succeed and the second to fail, for example with a transient provider startup error.

**Actual:** The loop correctly continues after individual failures (`NewSession.svelte:358`). It then opens successful sessions, shows a toast containing the failures and calls `onclose()` whenever at least one session succeeded. The form unmounts. Only provider/browser preferences were saved; failed members' configuration and remaining batch counts are not retained. Reopening New session requires reconstruction. Repeating the original full batch also creates successful members again.

**Intended:** Partial success should preserve a usable path to finish the remaining batch, with the failed members' submitted configuration intact and a clear separation from already-started sessions. The existing all-failed branch already keeps the sheet open for retry (`:376`), establishing the recovery expectation.

**Impact/severity:** Minor: successful work remains available and users can recreate the missing sessions, but a recoverable partial failure unnecessarily loses entered configuration and makes duplicate spawning easier. No claim that existing session data is lost.

**Fix direction:** Retain a result state in the sheet (or a recoverable batch draft), showing successes and failed members. Offer Retry failed using only failed requests and their captured configuration. Keep successful sessions available without automatically dismissing the only recovery surface. Root confirmed Codex owns this partial-retry logic and has forwarded the finding to Claude for coordination.

**Regression:** In an isolated browser fixture, submit two agents with nondefault values; succeed A and reject B. Assert A exists once, B's failure and configuration remain recoverable, and Retry failed sends exactly B's original request. Succeed B and verify the batch completes without duplicating A. Retain all-success and all-failed cases. No regression executed here.

## Forwarded, external-owned finding

### R4-U1-02 — Minor — conversation search presents an incomplete search as “No matches”

**Location:** `ui/src/modules/agents/conversation/ConversationView.svelte:413`–`:416` (searches only `conv.turns`), `:753`–`:757` (“Search this conversation” / “No matches”). Initial read is limited to 60 turns at `ui/src/lib/stores/transcript.svelte.ts:67`, `:235`; earlier turns load separately at `:274`.

**Steps:** Open a 120-turn conversation at its newest page. Use the conversation header's search button and enter a unique phrase present only in turn 1.

**Actual vs intended:** The search returns “No matches” from the loaded tail without indicating that older messages were not searched. Manually paging earlier changes the result. A user trying to recover an earlier decision can wrongly conclude it is absent. The interface should explain the searched scope and give a direct way to extend it; this does not require eagerly loading entire transcripts.

**Fix direction:** When `has_earlier`, disclose that matches cover loaded messages and expose a contextual Load earlier action, or implement a bounded full-history search. Keep definitive “No matches” for a complete search. The nearby “All changes in this conversation” control (`:716`–`:720`, `:744`) similarly derives its scope from loaded turns; include it in the same scope-label review rather than count another finding.

**Regression:** Seed a unique phrase only before the initial page. Verify no definitive whole-conversation absence claim; extend the scope and verify the phrase becomes reachable. Verify a fully loaded conversation can still report no matches. No test run here.

**Ownership:** Root confirmed this was forwarded to Claude owner A, who owns the loaded-search scope repair alongside ConversationView changes and search/filter vocabulary. This is evidence of a specific incomplete-search journey, not a second general placeholder-style finding or a competing edit reservation.

## Existing dependencies — no duplicate implementation findings

- **R4-C1-01, blocker:** History's “Resume” on a working session restarts the active PTY instead of opening it (`HistoryPage.svelte:145`). The explicit user intention is to continue/open the conversation; this interrupts current work without the ordinary restart warning. Covered by `iteration-4-correctness-1.md` and reserved to Codex; do not create a separate UX repair.
- **R4-C1-02, minor:** History's Open in Chat writes localStorage while SessionView reads the transcript view cache (`HistoryPage.svelte:160`, `SessionView.svelte:278`). Same correctness owner and regression.
- **Claude-owned:** Disconnected LiveStatus can continue presenting work as current (`ConversationView.svelte:843`); UI-control approval presentation, share confirmation, Rail history navigation and global command/shortcut consistency already appear in `/tmp/otto-design-iter4-findings.md`. Their source state is not credited as repaired here.
- **Performance partition 1:** Subagent retained-body lifetime/windowing is already tracked. No duplicate responsiveness/memory finding.

## Source journey matrix

| Journey | Evidence and conclusion | Execution |
|---|---|---|
| Arrive in Agents; session discovery fails; retry | `AgentsPage.svelte:38` waits for successful discovery before auto-opening; `:55`–`:64` exposes Retry and prevents unready stale panes. Prior failed-load issue has an explicit current-source repair. | Not run here |
| First-run optional skills fail, then launch | `FirstRunCoach.svelte:103`–`:115` distinguishes skills error; `:159`–`:182` keeps launch state on failure and uses daemon-delivered starter prompt. | Not run here |
| Create a mixed-success batch | `NewSession.svelte:300`–`:383`; R4-U1-01. | Not run here |
| Paste image; attempt Send; failed send; navigate between drafts | `Composer.svelte:68` captures session ownership; `:243`–`:264` gates pending uploads/duplicate sends and clears only the submitted unchanged draft after success. `:311`–`:324` attaches finished uploads to their owner. | Not run here |
| Conversation unavailable or failed read; terminal fallback / Retry | `ConversationView.svelte:650`–`:665`, `:793`–`:815`, `:866`: named unavailable states, Open terminal and explicit retry. Earlier-page loading and jump-to-latest are visible at `:819`, `:871`. Search scope remains R4-U1-02. | Not run here |
| Terminal disconnect / suspended session | `Terminal.svelte:524` schedules capped-backoff reconnect; `:2917`–`:2957` distinguishes suspended/reconnecting/disconnected/read-only and provides Resume/Reconnect controls. Native focus/socket behavior unverified. | Not run here |
| Handover prepare, failure, receipt | `Handover.svelte:105`–`:155` retains form on failed generate/send; `HandoverDeliveryPanel.svelte:27`–`:40` exposes saved brief, retry and explicit receipt before optional source archive. | Not run here |
| Palette search failure and returning to a module | `Palette.svelte:68`–`:109` owns abort/error state per request; router stores per-window module routes at `router.svelte.ts:68`–`:118`. No new defect asserted from these sampled paths. | Not run here |
| History open/resume | `HistoryPage.svelte:136`–`:162`, `SessionView.svelte:278`; existing R4-C1-01/02. | Not run here |

## Fixed-rubric score

| Dimension | /2 | Evidence and deduction |
|---|---:|---|
| Task completion/discovery | 1.6 | Source shows onboarding, auto-open, terminal fallback and recovery actions; History's open/continue path violates its task, and older conversation search is undisclosed. |
| Feedback/state clarity | 1.6 | Distinct terminal/read errors and upload/send state are present; partial-batch outcome is transient, search scope is incomplete and Claude's disconnected LiveStatus finding remains open. |
| Recovery/retry | 1.7 | Agents, transcript and handover have explicit retries; mixed-success creation loses the remaining request configuration (R4-U1-01). Real provider/network recovery matrix unexecuted. |
| Draft/scope/trust preservation | 1.2 | Composer ownership and unchanged-draft clearing are strong direct source evidence; R4-C1-01 can interrupt ongoing work from an ordinary open/continue action. Batch and search scope gaps also remain. |
| Executed end-to-end journeys | 1.0 | Shared verified green baseline `03f2bc3e` per PLAN and prior VERIFICATION; no current journey mapping or execution by this reviewer. Uses the mandated calibration, not zero and not a claim that these scenarios passed. |
| **Total** | **7.1/10** | Provisional source judgment. Existing blocker prevents acceptance irrespective of arithmetic. |

The same existing defect can explain deductions in different dimensions; it remains one tracked repair. Initial score already uses the shared execution calibration, so there is no alternate uncalibrated total to replace.

## Evidence limits and next acceptance work

Omitted live provider CLI behavior; native macOS keyboard/focus/window restore; split/tiled/phone task completion; actual clipboard/image upload; reconnect across daemon restart; read-only remote sharing; rendered light/dark/RTL; assistive technology; full creation/provider/status permutations. Reading a retry button is not evidence that a retry journey completed. No whole-app UX claim is made.

Map root's execution to named cases: partial-batch retry; earlier-page search scope; History working/idle/inactive/on-disk actions with warmed view cache; failed composer send with pending image and navigation; Chat → terminal approval → Chat; workspace discovery failure/retry; terminal reconnect. Preserve this provisional score and append executed outcomes and calibrated rescoring after repair/integration.
