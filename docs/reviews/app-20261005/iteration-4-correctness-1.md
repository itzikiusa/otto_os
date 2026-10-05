# Iteration 4 correctness review — partition 1

**Verdict: Block — 1 blocker, 1 minor confirmed by source trace.** Provisional score **7.0/10**. This was a source-only review at baseline `03f2bc3e` on `fix/app-review-20261005`; no tests, builds, servers, browser journeys, or native actions were executed. No source files changed.

## Independent plan check

The plan has suitable isolation, ownership, affected-consumer gates, independent review, explicit scoring dimensions, and honest runtime limits. The material detail still needed before acceptance is a concrete journey/state matrix for each partition: this partition needs running/working/idle/reconnectable/exited History actions, view preferences, transcript disconnect/paging, and terminal identity switches. Broad partition names plus aggregate gate counts cannot establish those states were exercised. Add these rows with test names and outcomes when implementing the findings; do not treat this source-only report as executed coverage. No additional approval or redesign is needed for this plan refinement.

Coordination read: `/tmp/otto-app-review-coordination-20261005.txt`. Claude owns visual/a11y/copy/markup/CSS, notifications deep links, swarm bulk `allSettled`, and URL selection in Workflows/Proof/Swarm/Scheduled/Loops/Product. Root has been told these two behavioral fixes need the HistoryPage handlers reserved with Claude's implementer A. No behavioral path was edited by this reviewer.

## Ranked findings

### R4-C1-01 — blocker — History's resume action restarts a working session and kills its active turn

**Location:** `ui/src/modules/agents/history/HistoryPage.svelte:145` (also `:204`, `:210`, `:384`); contract mismatch at `ui/src/lib/api/types.ts:9494`.

**Intended behavior:** History's action opens a live session; only inactive sessions need restart. The function's own comment describes import → restart for exited/reconnectable → open in Chat. Ordinary user-facing restarts expressly warn about interrupting working agents (`ui/src/lib/stores/workspace.svelte.ts:1620`).

**Confirmed by hand trace, not execution:**

1. A Claude session A has a provider session ID and is producing output. The manager persists `SessionStatus::Working` at `crates/otto-sessions/src/manager.rs:5692`, `:5716`.
2. History's page route returns that status verbatim using `session.status.as_str()` (`crates/otto-server/src/routes/history_page.rs:178`), and sets `resumable=true` for this non-exited session with a provider session ID (`:180`). TypeScript's `HistoryStatus` union omits `working`; this does not filter the actual JSON.
3. History treats only `running` or `idle` as live. A's action is enabled because `resumable=true` but says “Resume in Otto.” Clicking it enters the condition at `HistoryPage.svelte:145`, so it calls `ws.restartSession(A)` instead of merely opening A.
4. `restartSession` posts directly to `/sessions/A/restart` (`workspace.svelte.ts:1609`), bypassing `requestRestart`'s working-session confirmation. The route calls manager restart directly (`crates/otto-sessions/src/http.rs:520`). `restart_locked` removes A's live handle and kills it (`manager.rs:5245`), then starts a replacement process.

**Actual vs expected:** Opening the already-working conversation interrupts its current agent process without the working-session warning. Expected: navigate to A's Chat without any restart request or process replacement. This is a real active-work-loss path, not merely incorrect status copy.

**Fix direction:** Include `working` in History's status contract/types and treat running/working/idle consistently as live in the action handler, labels, menu eligibility, detail action, filters/status rendering. Prefer one shared live-status predicate. Restart only explicitly inactive states; review the stale-row branch so an inactive History snapshot cannot silently restart a session that became live after listing. Update the authoritative History contract alongside the type if its enumeration omits this state.

**Regression idea:** Return a real History payload with `status:'working'`, a session ID and `resumable:true`. Exercise both the row menu and detail action; assert navigation to A and zero restart requests. Include running, idle, reconnectable and on-disk rows to prove the inactive resume/import paths still work. Root should record red/green evidence; none was run here.

### R4-C1-02 — minor — History's Open in Chat action loses to a cached Terminal preference

**Location:** `ui/src/modules/agents/history/HistoryPage.svelte:160`.

**Intended behavior:** `openInChat` explicitly promises to open the selected session in Chat, and the enclosing resume path makes this its last step.

**Confirmed by hand trace, not execution:**

1. Open A and explicitly select Terminal using its view switch. `SessionView.svelte:294` calls `transcript.setView(A, 'terminal')`, which sets the in-memory `views[A]` and localStorage.
2. Navigate to History and use Open in Otto on A while A is idle. `openInChat(A)` directly writes localStorage to `chat` at line 160, then navigates to A at line 165. It does not update the transcript store.
3. On the destination, `SessionView.svelte:278` reads `transcript.view(A)`. That method returns the cached value immediately (`ui/src/lib/stores/transcript.svelte.ts:605`), before reading localStorage.

**Actual vs expected:** A opens in Terminal, contrary to the action's Chat intent. Reloading can make the preference appear correct, which masks the same-window failure.

**Fix direction:** Use `transcript.setView(sid, 'chat')` in `openInChat`, so reactive state and persisted preference change together. Do not create another preference cache.

**Regression idea:** Set A's view to Terminal through the store, invoke History's open action, and assert both `transcript.view(A)==='chat'` and the destination ConversationView is visible without reload. Test the previously-uncached case too. No test was executed here.

## Traced areas without an additional confirmed finding

- Router lazy-load commits use `commitSeq`; competing guarded navigation uses `guardSeq`; malformed percent escapes have a nonthrowing decoder; current-hash history movement avoids leaving `navigating` stuck.
- Transcript leases, hidden-document aborts, disposed read epochs, bounded recovery scheduling, reconnect gaps without overlapping turns, earlier-page cursor ownership, and deliberate history-cap holds were read and traced. The existing no-overlap recovery repair replaces the visible window rather than retaining an unreachable middle gap. This is not a claim that every provider delta/race was validated.
- Composer ownership captures session ID, preserves new text/files during an in-flight send, and serializes sends per session. The current source has protections absent from early historical reviews; those older findings were not reopened.
- Session restart/archive serialization and authoritative status events were sampled; current manager comments and tests establish per-session locking and superseded-handle protection. The History issue above bypasses user intent despite that correct serialization.

## Question retained below the confirmed-finding bar

`ui/src/lib/components/Terminal.svelte:2624` closes and drops the old socket without clearing its callbacks in the non-parking session-switch branch, while `connect()` resets `closedByUs=false`. Its `onclose` at `:1032` has no `s===sock` guard. A delayed close from A would therefore alter B's state and schedule an extra reconnect. Source tracing establishes that branch's risk, but the sampled production callers either park, key by session ID, or conditionally unmount their terminals; I did **not** complete a concrete caller trace that preserves this exact non-parking instance across A→B. Treat this as a targeted runtime/caller investigation, not a third confirmed bug. A fake-socket component test with delayed A close would resolve it cheaply if a caller is established.

## Fixed rubric — provisional, source-only

| Dimension | Score /2 | Evidence and deduction |
|---|---:|---|
| Contract/data integrity | 1.0 | Real `working` status omitted from History's model causes unintended process replacement (R4-C1-01). Other sampled History fields and bounded candidate mapping were traced. |
| State/concurrency ownership | 1.7 | Transcript epochs/leases and composer ownership have concrete guards; view cache diverges from History's direct storage write (R4-C1-02). Non-parking terminal-switch ownership remains a stated question. |
| Boundary/error behavior | 1.7 | Router malformed hashes, read abort/error branches and History stale page fencing inspected. Full provider/status matrix and server error journeys unexecuted; working-status action branch is materially wrong. |
| Persistence/recovery | 1.8 | No-overlap transcript recovery, earlier cursors, draft storage and session restart/archive paths sampled. Cross-daemon/provider recovery and native relaunch not executed. |
| Executed regression coverage | 0.8 | Relevant existing transcript test source was inspected, but **zero** tests or journeys were executed by this role. Prior baseline CI is historical evidence only, not coverage of the newly traced failures. Root must add independently recorded red/green and browser evidence. |
| **Total** | **7.0/10** | **Provisional review judgment; blocker prevents acceptance regardless of arithmetic.** |

## Inspected files and explicit omissions

Primary reads: `AGENTS.md`; current `PLAN.md`; correctness-review skill; `ui/src/lib/router.svelte.ts`; `ui/src/shell/App.svelte`; `ui/src/modules/agents/AgentsPage.svelte`; `SessionView.svelte`; `history/HistoryPage.svelte`; `history/history.svelte.ts`; `conversation/Composer.svelte`; `conversation/api.ts`; `ui/src/lib/stores/transcript.svelte.ts`; `transcriptLifecycle.ts`; selected session/navigation mutation paths in `workspace.svelte.ts`; transcript routing in `ui/src/lib/events.svelte.ts`; selected attach/snapshot/parking/reconnect/switch paths in `ui/src/lib/components/Terminal.svelte`; `crates/otto-server/src/routes/history_page.rs`; selected legacy History paths in `routes/transcript.rs`; `crates/otto-state/src/history_page.rs`; lifecycle/restart/status paths and relevant test declarations in `crates/otto-sessions/src/manager.rs`; restart HTTP route; History API types. Sampled terminal caller markup in workflow, skill-review, git-review, personal-agent, canvas and browser panels to check switch reachability. Inspected existing `ui/unit/*transcript*` test names and selected prior partition-1 report references only to avoid reopening closed defects.

Omitted: full manager/PTY/WebSocket implementation, complete transcript parsers and disk indexer, every pane/tile/detachable-window interaction, real provider startup/resume, native keyboard/WebGL behavior, multi-window/device behavior, every history filter/page combination, and runtime verification throughout. No exhaustive correctness claim or 9.8 acceptance claim is made.
