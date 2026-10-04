# Iteration 1 — UX partition 4

Scope: workflows, Goal Loops, Swarm, Scheduled Tasks, Mission Control, and MCP. **Three uncovered findings: two major, one minor.**

Evidence is read-only source tracing, not runtime reproduction. No builds, tests, servers, daemon requests, or source edits were performed. Read `AGENTS.md`, the design guidelines and review checklist, the partition-4 correctness/performance reports, and `/tmp/otto-app-review-coordination-20261004.txt`. Compared the assigned UI paths against the design worktree's diff from `a16f4c71`; the findings below remain outside its fixes. This report is the only authored file.

## UX4-01 — [major] Replace policy import understates its destructive scope and can omit confirmation entirely

**Location:** `ui/src/modules/mcp/PoliciesTab.svelte:111`, with the visible list populated at `:37`; actual replacement at `crates/otto-mcp/src/http.rs:880`.

**Scenario:** Workspace A has no policies or global rules; workspace B has several policies. An administrator opens MCP Policies in A, imports a valid rule array, selects “Replace all existing rules,” and clicks Import. A's list is empty, so the confirmation branch is skipped. B's policies are deleted even though none appeared in A's view. If A has one visible rule instead, the confirmation says it will replace “all 1 existing rule,” but B's rules are still deleted as well.

**Source trace:** `cpPolicies(wsId)` supplies the workspace argument (`ui/src/lib/api/mcp.ts:98`). `McpPolicyRepo::list(Some(ws))` returns only global rules plus that workspace, while `list(None)` returns every workspace's rules (`crates/otto-state/src/mcp_control.rs:900`). The confirmation uses this scoped `policies.length` both as its gate and its deletion count. The import POST supplies only the imported policies and `replace`; the backend explicitly lists `None` and deletes every returned rule before creating the imported set. The observed mismatch is between the user's visible/confirmed deletion scope and the actual operation, independent of authorization checks.

**Impact:** A routine import removes other workspaces' policy configuration without disclosing that scope. An empty current workspace does not imply an empty application-wide ruleset.

**Smallest fix:** Preserve the existing global-import contract but fetch the full affected ruleset before a replace operation. Always confirm replacement explicitly as applying to every workspace, using the full count and affected workspace summary; a failed scope fetch must block replacement and allow retry. Alternatively, make replacement explicitly scoped end to end, updating the API contract and UI together. Do not merely change the label while retaining the scoped-count confirmation gate.

**Regression:** Seed rules only in B, open A, and assert that Replace shows the global effect before any POST. Cancel must preserve B. Repeat with rules in A, B, and global scope to verify the count; reject the scope fetch and assert there is no replace POST.

**Ownership check:** Claude's `PoliciesTab.svelte` diff changes load-state rendering and styles, not the import branch.

## UX4-02 — [major] Selecting another Mission Control item silently discards the current edit

**Location:** `ui/src/modules/mission-control/MissionControlPage.svelte:59`; draft replacement at `ui/src/modules/mission-control/WorkItemDetail.svelte:59` and `:70`.

**Scenario:** Open a work item, choose Edit, and write a substantial goal or result summary. Click another item in the list or graph to inspect its context. The draft disappears without Save/Discard/Keep editing. Reopen the first item: its previous server value returns. Closing the detail pane has the same result.

**Source trace:** Edit controls bind to component-local `editGoal`, `editResult`, and `editRisk` (`WorkItemDetail.svelte:271`). Both list and graph pass selection directly to `select` (`MissionControlPage.svelte:382` and `:384`), which immediately replaces `selectedId` and uses `router.replace`. The detail's ID effect loads the new item; `load()` explicitly sets `editing = false` when the ID changes and then reseeds all edit fields from the server. Closing invokes `select(null)` (`:409`) and unmounts the detail. Neither path checks for changed fields or retains a draft. The existing protection against live-event reloads while editing does not protect these navigation paths.

**Impact:** Ordinary item exploration loses manually authored work with no recovery path. This violates the documented protection for discarding nontrivial edits in `docs/design/guidelines/patterns.md` §7.

**Smallest fix:** Expose a dirty/leave decision from the detail and await it before every selection/close that replaces the draft. Reuse the shared unsaved-change confirmation, compare actual fields so clean edits do not prompt, and retain the current selection and contents on Keep editing. Register the same decision for module navigation. A router guard alone cannot protect this selection path because `select` mutates state before `router.replace`, which intentionally bypasses guards.

**Regression:** Change each editable field, select another list/graph item, and verify Keep editing preserves the first item and draft. Verify Discard moves to the target, Save preserves values when reopened, clean navigation does not prompt, and close/module navigation has the same protection.

**Ownership check:** Claude's `WorkItemDetail.svelte` diff changes approval controls and styles; its `MissionControlPage.svelte` change is styling. Neither adds draft protection.

## UX4-03 — [minor] A failed Resume disappears after extending a Goal Loop budget

**Location:** `ui/src/modules/loops/LoopDetail.svelte:151`; the only error display is inside the conditional modal at `:354` and `:365`.

**Scenario:** An exhausted loop needs more budget. Enter larger limits and choose Extend & resume. The limits PATCH succeeds, but the following Resume request fails, for example because the daemon connection drops. The dialog has already closed; no error or partial-success explanation appears. The loop stays stopped despite the completed-looking combined action. Reopening Extend clears the stored error.

**Source trace:** `extendAndResume` awaits `loops.updateLimits`, sets `extendOpen = false`, and only then awaits `loops.resume`. Its catch assigns `extError`, which is rendered solely inside `{#if extendOpen && loop}`. `openExtend` clears that error. The store merges the successful limits update (`ui/src/lib/stores/loops.svelte.ts:165`) but the lifecycle call propagates a rejected Resume, so the catch is reachable without any failed budget update. There is no toast or durable page-level result for this partial completion.

**Impact:** The user cannot tell that the budget was saved but execution did not resume, and must infer recovery from the unchanged status. This is narrower than loss of work, hence minor severity.

**Smallest fix:** Keep the modal open until Resume succeeds, and explain partial success on Resume failure: the budget is saved, the loop has not resumed, and Resume can be retried without requiring another extension. Ensure closing a pending modal cannot swallow its final failure.

**Regression:** Resolve the limits PATCH and reject Resume. Assert a visible partial-success error, unchanged stopped state, preserved new limits, and an available retry. Verify retry resumes and closes the dialog only after success; also cover a failed limits PATCH.

**Ownership check:** Claude's Loop Detail diff moves destructive actions into overflow and changes styles; the extension sequence is unchanged.

## Coverage and limits

Sampled workflow graph/instructions save and run entry, internal workflow navigation/history; Goal Loop lifecycle/recovery and detail state; Swarm agent/goal editors and standing-goal settings; Scheduled Task create/edit/close/run/report entry points; Mission Control selection/edit/approval flow; and MCP policy import/editor/evaluation surfaces. This was a finite source review, not exhaustive behavior coverage.

Excluded the existing workflow launch/version ownership races, swarm lifecycle/concurrency defects, MCP approval-owner lookup, and all partition-4 performance findings. Also excluded the design effort's load/error states, approval presentation, Scheduled Task Run-now delivery confirmation, copy, visuals, keyboard and accessibility changes. Regression cases above are proposed verification, not tests run or passing claims.
