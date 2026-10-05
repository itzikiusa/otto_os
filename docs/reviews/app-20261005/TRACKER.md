# Current checkpoint — 2026-10-05 09:03 local

Iterations 4 and 5 findings have repairs and recorded follow-up evidence. The twenty-role final iteration-6 review is closing; [SCORES.md](SCORES.md) holds its latest published values and remaining deductions. This effort will deliver one continuation PR. No deployment is authorized or performed.

Main through PR82 (`9ab214dd`) is integrated without losing the concurrent design changes. The four iteration-5 defects (departed History resume, departed/reopened Product publication, Personal Agents list ABA ownership, and stale/duplicate scheduled admission) have targeted failing-then-passing evidence. Final acceptance additionally repaired History navigation awaiting a page chunk, Audit keyboard/touch detail disclosure and Product initial deep-link selection during workspace startup.

Final integrated UI check: 0 errors / 0 warnings; 1,335 unit passes; production build and unchanged bundle budget passed. Focused browser batches and exact failed-then-repaired Rust checks are recorded in [VERIFICATION.md](VERIFICATION.md), including failures rather than relabeling earlier runs.

Synthetic N=0/1/3/5 plus recovery completed. The 15-minute sustained run stopped at 414 seconds under the unchanged host-load safety limit. Native attribution and representative non-session workloads remain incomplete; see [PERFORMANCE.md](PERFORMANCE.md). The 9.8-in-every-partition target is not met merely by completing three iterations.

The entries below are historical discovery/repair checkpoints, not the current open-defect list. Later review dispositions and named execution results supersede their original “reserved” and “pending” states.

---

## Historical checkpoint (2026-10-05 04:20 UTC)

Iteration 4 repair acceptance remains in progress; all twenty baseline reviews and five implementation roles are complete. New broad discovery is frozen until this checkpoint closes. Current provisional scores: correctness **8.72 / minimum 8.3**, performance **8.40 / minimum 8.0**, UX **8.30 / minimum 7.9**. See iteration-4-provisional-scores.md for dimensions, evidence and deductions. Design's same-reviewer static mean is **9.96 / minimum 9.80**; integrated/rendered acceptance remains separate. The target stays 9.8 in every partition/lens.

Latest backend verification: server approval/scheduling/admission 30/30; Design library 128 passed/1 ignored; broker replay 6/6; recap revision library 5/5; grouped state/Git/Product nextest 106/106. The latter includes all five Git raw/render/capture budget regressions and Product byte-projection HTTP tests. Current history HTTP/50k and recap authenticated-route checks are running; MCP paging final execution remains queued. Earlier failures and their named repairs remain in VERIFICATION.md.

RoomRecap identity is now repaired with mounted WebKit RED→GREEN and independent review; all12 authored desktop cases have passing executions across the initial run and two assertion-only reruns. Combined current UI/Rust gates, mounted journeys, light/dark captures and representative CPU/RAM/sustained measurements remain outstanding. Git exact omitted-file counts retain linear disk reads. No whole-app latency/memory improvement inferred from protocol counters.

Claude released the heavy slot at 07:05 local; ours remains reserved for serial acceptance. PR79 merged 28cd216c, PR80 merged 06154492, PR81 awaits green CI/merge. Integrate after preserving our coherent checkpoint; apply the supplied Vault backlinks presentation patch afterward. Our final iterations and single continuation PR/green merge remain ahead. No deployment. Historical entries below preserve chronology and are superseded by named later dispositions.

# Iterations 4–6 findings tracker

Source baseline `03f2bc3e`. Findings need concrete traces/reproductions before implementation. Tests pending are not passing. See PLAN.md and SCORES.md for rubric and acceptance.

| ID | Finding | Ownership | State |
|---|---|---|---|
| R4-C1-01 | History Resume treats working session as restart, killing active PTY (blocker) | Codex role1; HistoryPage resume + HistoryStatus type | Source-confirmed; report ready; reserved with Claude |
| R4-C1-02 | History Open in Chat bypasses transcript view cache (minor) | Codex role1; HistoryPage open handler | Source-confirmed; report ready; reserved with Claude |
| C4-2-01 | Broker replay drops/corrupts binary key/value/header and null value semantics (blocker) | Codex role2; broker service/Kafka | Source-confirmed; report ready |
| C4-2-02 | Delayed DB guarded-write prompt names current connection B while retry writes captured A (blocker) | Codex role2; database store | Source-confirmed; report ready; reserved with Claude |
| C4-2-03 | Broker preview slices UTF-8 at arbitrary byte offset and can panic (major) | Codex role2; broker service | Source-confirmed; report ready |
| C4-2-04 | API late loader/mutation responses publish old workspace data into current workspace | Codex role2; API store | Source-confirmed; report ready; reserved with Claude |
| C4-2-05 | Environment Save response replaces edits made while Save was pending | Codex role2; EnvironmentsView | Source-confirmed; report ready; reserved with Claude |
| R4-C3-01 | Concurrent Design metadata patches lose acknowledged independent edits (blocker) | Codex role3; Design service/store | Source-confirmed; report ready |
| R4-C3-02 | Browser delayed tab creation crosses workspace ownership (major) | Codex role3; Browser store | Source-confirmed; report ready; reserved with Claude |
| R4-C3-03 | Browser navigation persists another selected tab's title (major) | Codex role3; Browser store | Source-confirmed; report ready; reserved with Claude |
| R4-C3-04 | Excalidraw restore omits persisted background/grid state (minor) | Codex role3; ExcalidrawCanvas | Source-confirmed; report ready; reserved with Claude |
| R4-C4-01 | Workflow graph/version/snapshot publication can execute a different graph (blocker) | Codex role4; workflow repository/routes | Source-confirmed; report ready |
| R4-C4-02 | Old scheduled completion consumes newly retimed one-shot (major) | Codex role4; scheduler repository/engine/routes | Source-confirmed; report ready |
| R4-C4-03 | Fired workflow trigger does not rearm on retiming (major) | Codex role4; workflow triggers | Source-confirmed; report ready |
| R4-C5-01 | Parallel personal-agent schedule/run loads overwrite other entries (major) | Codex role5; personalAgents store | Source-confirmed; report ready; reserved with Claude |
| R4-C5-02 | Stale Proof filter response replaces current rows/cursor (major) | Codex role5; Proof store | Source-confirmed; report ready; reserved with Claude |
| R4-C5-03 | Pending Proof refresh reopens closed detail (minor) | Codex role5; Proof store | Source-confirmed; report ready; reserved with Claude |
| P1-R4-01 | Collapsed subagent bodies accumulate outside active budgets; child paging mounts unbounded turns (major) | Codex role1; transcript store/SubagentCard | Sized source finding; runtime magnitude pending; reserved with Claude |
| R4-P2-PERF-01 | Aggregate Working diff retains every untracked patch before capping (major) | Codex role2; Git local diff | Sized source finding; runtime pending |
| R4-P2-PERF-02 | Workbench history reloads/mounts lifetime revision list (major) | Codex role2; Workbench repository/API/HistoryPanel | Sized source finding; runtime pending; reserved with Claude |
| R4-U1-01 | Mixed session creation drops failed requests' configuration (minor) | Codex role1; NewSession submit recovery | Source trace; reserved with Claude |
| R4-U1-02 | Search gives whole-conversation absence claim over loaded tail only (minor) | Claude; ConversationView scope feedback | Forwarded for external implementation |
| R4-U2-01 | Import completion runs arbitrary unsubmitted SQL (blocker) | Codex role2; ImportDialog | Source-confirmed; reserved; runtime regression pending |
| R4-U2-02 | Save request retry duplicates already-created collection (minor) | Codex role2; SaveRequestDialog | Source-confirmed; reserved; preserve Claude copy |
| R4-U2-03 | Recovery Refresh clears history error without reloading history (minor) | Codex role2; RecoveryTools | Source-confirmed; reserved |
| P3-01 | Draft Overview eagerly loads all imported transcript bodies (major) | Codex role3; Product summary/body/find API and UI | Sized source trace; reserve data handlers; preserve cross-history find |
| P3-02 | Large shared diff fallback mounts all lines as changed (major) | Codex role3; DiffView | Sized source trace; algorithm/windowing reserved |
| R4-D1-01 | Session tabs lack RTL/reorder keyboard equivalents (minor) | Claude | Forwarded; source/runtime recheck pending |
| R4-D1-02 | Expanded file-preview error lacks Retry (minor) | Claude | Forwarded; source/runtime recheck pending |
| R4-D2-01 | Vertical result field actions inaccessible by keyboard (major) | Claude | Forwarded; source/runtime recheck pending |
| R4-D2-02 | Schema comparison selectors omit selection/version semantics (minor) | Claude | Forwarded; source/runtime recheck pending |
| P4-01 | Workflow Versions loads every full graph/instructions snapshot (major) | Codex role4; version repository/API/drawer | Sized source finding; reserved, preserve Claude URL/layout changes |
| P4-02 | Shared node-body cache has count cap but no byte cap (minor) | Codex role4; runProgress SharedNodeBodies | Sized source finding; byte/LRU reservation |
| R4-P5-01 | Completed recaps repeatedly read/transfer unchanged event bodies (minor) | Codex role5; RecapPanel/archive revision loading | Sized source trace; preserve external/manual update visibility |
| R4-UX3-01 | Publish may send a newer version than the successful preview (major) | Codex role3; immutable publication binding | Source-confirmed; reserved; isolated outbound payload regression required |
| R4-UX3-02 | Single-account destination lookup failure lacks Retry (minor) | Codex role3; PublishDialog scoped destination loading | Source-confirmed; reserved |
| R4-UX3-03 | Rejected Canvas assist clears retryable prompt (minor) | Codex role3; ConversationPanel/editor generate results | Source-confirmed; reserve all three editor handlers and scene ownership |

Canonical IDs/severity/traces are in reviewer reports. Role1 implementation is underway after recorded UI/Rust red evidence; no green acceptance yet. Other fixes await implementation. External known design patterns remain in their reports and are not duplicated as separate Codex repairs.

Claude owns iteration-4 visual/a11y/copy fixes and announced URL selection work in Workflows/Proof/Swarm/Scheduled/Loops/Product, plus notifications deep links and swarm bulk allSettled. External reports: `/tmp/otto-design-iter4-findings.md`, `/tmp/otto-design-review-brief.md`; coordination `/tmp/otto-app-review-coordination-20261005.txt`.

Claude reports source repairs for R4-D1-01/02, R4-U1-02 and R4-D2-01/02 in `b751ca6f` on `fix/design-iter-4`: tab keyboard/RTL/reorder and sibling close controls; preview Retry in both hosts; loaded-only search scope/earlier affordance; keyboard tree/field menu; schema A/B names/pressed state/check mark. External UI gates reportedly pass 0/0 check, 1,091 units, build and budget. Full E2E plus baseline comparison running. These are external reported results, not yet integrated or independently verified here; baseline deductions remain until the next source/acceptance pass.
# Additional partition-3 design handoff

Source review `iteration-4-design-3.md` scores 8.8 provisionally, with no new runtime execution. R4-D3-01 (3D/Vault tree keyboard model) and R4-D3-03 (Switcher active-result semantics) forwarded to Claude. R4-D3-02 extends the failure-as-empty pattern: Switcher/Tags lookup error propagation, pending/error creation guard, retry and stale tags are reserved to our role3 in `vault.svelte.ts`, `Switcher.svelte`, and `TagsPanel.svelte`. Coordinate Switcher accessibility markup with Claude. These findings are open; no score increase from assignment alone.
# Final UX baseline findings

All five UX reports are complete: mean 7.36/10, minimum 7.1. R4-UX4-01 and R4-UX4-02 are major draft-loss cases: Scheduled Task/Goal Loop navigation bypasses local discard guards, and a pending schedule save closes over newer edits. Role4 owns behavior, preserving Claude's selection markup. R4-U5-01 is a major undrained Insights model-save queue after a schedule toggle; R4-U5-02 is minor avoidable loss of an uncopied one-time token when another token is created. Role5 owns those settings handlers. Exact UI reservations were appended to the shared note and sent to Claude. All remain open pending repairs and execution.

Role1 independent source recheck found spec conformance and no additional confirmed behavior defect. Root's latest focused run is 35/35 green; rendered journeys and explicit budget/workspace coverage gaps remain pending. No rescore yet.
# Partition4 design handoff

Independent D4 source score 8.2. Forwarded all additional locations to Claude: OrgTree hierarchy/keyboard semantics (same systemic tree pattern as D3); AgentGraph hover-only, unclamped session chooser; IterationRow pending/retry state; AuditTab accessible success/failure and phone field labels (extends existing MCP label pattern). Exact traces and acceptance are in `iteration-4-design-4.md`. These are external presentation repairs, still pending integration and runtime verification.
# Role2 independent recheck follow-ups

`iteration-4-role2-ui-recheck.md` confirms the six original UI triggers repaired in source, but requests changes for R4-R2-UI-01 (major: cancel/unmount during collection creation still starts request save) and R4-R2-UI-02 (minor: environment A→B→A during Save leaves stale fields displayed clean). Both are accepted after tracing real lifecycle/selection callers; role2 is adding regressions before repair. The 75/75 handler/store result does not close mounted lifecycle acceptance or raise the partition score.

Claude acknowledged D3/D4/D5 presentation findings in the coordination note; those repairs are assigned to `fix/design-iter-5`, worktree `/Users/itziklavon/claude_ade-design5`, stacked on iteration4. AccountPicker behavior and adjacent pending/error markup are wholly ours to avoid collisions. Claude still holds the heavy slot for iteration4 desktop testing.
# Role2 UI follow-up disposition

C2's independent follow-up resolved R4-R2-UI-01/02 in source after inspecting synchronous close/destruction and editor-generation reconciliation. Root's current focused suite is 80/80 green. UI check/browser acceptance remain open; backend broker/Git/Workbench work is now in test-authoring phase. No partition rescore yet.
# Role3 independent UI recheck follow-ups

`iteration-4-role3-ui-recheck.md` confirms the original UI repairs in source, then requests R4-R3-UI-01 (major: closed reader page can republish on late success/failure) and R4-R3-UI-02 (minor: Down during pending lookup leaves selection −1 so Enter does nothing). Root read and accepted both traces; role3 is adding targeted regressions before repair. Backend metadata, exact publication binding and content performance remain separate open tasks. No score increase from 39 passing tests.
# Role3 UI follow-up disposition and next implementation wave

Independent C3 follow-up resolved its closed-reader and pending-selection findings in source, with root's 18/18 regression result attributed separately. Backend metadata/publication/transcript/diff work remains assigned. Role4 has begun test authoring for automation draft protection/cache behavior, then atomic workflow/scheduling publication. Role2 is authoring broker/Git/Workbench backend regressions. Resource ceiling remains two shell-active roles and Claude's exclusive heavy slot.
