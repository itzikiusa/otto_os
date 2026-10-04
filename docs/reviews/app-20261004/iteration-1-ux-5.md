# Iteration 1 — UX partition 5

Two actionable findings, both **major**. Evidence is source-only: explicit user-flow traces, not browser/runtime reproductions. No builds, tests, servers, external actions, or application-source changes were performed.

Read `AGENTS.md`, the design overview/checklist and interaction patterns, partition-5 correctness/performance reports, other current finding headings and relevant draft-loss reports, and `/tmp/otto-app-review-coordination-20261004.txt`. Checked the separate design worktree's scoped committed/uncommitted diff; its changes to the affected personal-agent files are copy, focus styling, and tab keyboard behavior, not the recovery fixes below.

## UX5-01 — Major: Personal-agent document and autonomy drafts disappear on normal tab navigation

**Locations:** `ui/src/modules/personal-agents/AgentPage.svelte:67`, `:453`, `:636`; `ui/src/modules/personal-agents/AgentDocuments.svelte:18`, `:85`; `ui/src/modules/personal-agents/AgentAutonomy.svelte:68`, `:259`.

**Scenario:** Open a personal agent's Context tab, choose Edit, and write several paragraphs or add file references. Click Memory, Activity, or Back to Personal Agents before saving; return to Context. The draft is gone without a decision. The same happens to edited standing goals/rules/budget on Autonomy, including edits kept after a failed Save. Normal navigation silently performs what the explicit Discard button would otherwise do.

**Evidence:** Documents binds the textarea to component-local `draft`; only `save()` persists it. `AgentPage` changes the route through `goTab()`, conditionally unmounts the current tab, and keys Documents by agent/tab. Neither the document/autonomy components nor their page registers a router leave guard or keeps a draft outside the component. Autonomy already calculates `dirty` and presents Save/Discard, but uses that state only for controls. Returning mounts fresh components and loads the saved server version. The shared `guardUnsaved` exists at `ui/src/lib/leaveGuard.ts:24` and protects other editors; nothing global discovers these drafts automatically. The design guideline's Discard rule requires a decision before losing more than trivial edits (`docs/design/guidelines/patterns.md:227`).

**Smallest fix:** Register `guardUnsaved` within Documents for an edited draft different from the loaded content, and within Autonomy using its existing `dirty`. Include pending saves in the leave decision or keep navigation blocked until settlement so failed in-flight saves remain recoverable. Because these tab changes use `router.go`, component-level guards protect ordinary tab, back, and module navigation without changing tab styling. Keep editing must leave both the route and values intact; successful Save/explicit Discard must clear the dirty state.

**Verification:** Edit Context, attempt another tab and module navigation, choose Keep editing, and assert exact text/reference preservation. Choose Discard and verify intentional loss. Repeat for an Autonomy rule and budget, after a rejected Save, and while a delayed Save is pending. Unchanged and successfully saved forms must navigate without an unnecessary prompt.

## UX5-02 — Major: Retrying failed template setup creates another agent instead of completing the first

**Locations:** `ui/src/modules/personal-agents/AgentEditSheet.svelte:135`–`:149`; `ui/src/lib/stores/personalAgents.svelte.ts:112`, `:144`; `crates/otto-server/src/routes/personal_agents.rs:291`; `crates/otto-state/src/personal_agents.rs:497`.

**Scenario:** Create an agent using a template. The agent-create request succeeds, but the subsequent schedule-create request fails before committing, for example because the daemon becomes unreachable. The sheet stays open and says “Couldn’t save the agent.” Retry Save once service recovers. Otto creates a second agent, attaches the schedule to that new agent, and leaves the first enabled agent behind without its promised template schedule. The user cannot finish setup of the first agent from this retry path.

**Evidence:** `save()` calls `personalAgents.create()` and then `createSchedule(created.id, …)` as separate awaited mutations. The resulting agent ID is a local variable discarded on error. The catch covers both requests and reports an undifferentiated agent-save failure. The input `agent` remains null, so the next attempt takes the create branch again. The store's `create()` returns the persisted agent after list refresh; there is no rollback. The server POST creates a new record, and the repository allocates a fresh ID on each call. Agent names are not unique (`crates/otto-state/migrations/0112_personal_agents.sql:10`), so this retry is a valid second creation rather than an update or deduplicated request.

**Smallest fix:** Retain the successfully created ID and the remaining setup stage in the sheet. Report “Agent created; schedule could not be added,” and let retry complete schedule setup against that same agent. Do not silently delete the already-created agent. If the schedule request may have committed before its response was lost, reconcile its result or use an idempotency key before retrying that mutation.

**Verification:** Make create-agent succeed and the first create-schedule fail before commit. Retry from the same sheet; assert exactly one agent POST, the same agent ID on both schedule attempts, truthful partial-success feedback, and one completed agent/schedule. Also cover a schedule response lost after commit so retry does not add a duplicate schedule.

## Coverage and limits

Substantive new traces: personal-agent document/autonomy navigation, their local save/recovery state, template setup and backing create APIs. Sampled other assigned surfaces: settings login and settings backup/import controls; connection-export request state; plugin install/toggle/remove controls; Assistant composer draft restoration/attachment flow; room-chat pending nonce/retry behavior and recap setup; Home Usage request ownership; Insights run/completion polling; Share OTP/re-send recovery; History selection/resume; AWS log-group loading; Kubernetes resource-drawer ownership/permissions; Proof artifact submission. Sampling is not an exhaustive clean bill of health for those modules.

Excluded duplicates: canonical Assistant approval/cancel/takeover, plugin reinstall lifecycle, Athena history region, Assistant memory/reconnect bounds, Insights archive polling cost, and the parent's runtime-proven Kubernetes backfill retry loop. Home error-state and cloud/production confirmations already assigned to the separate design effort were not reasserted. Visual, accessibility, and copy-only issues remain with that owner. No runtime evidence or light/dark screenshots were collected for this source-only pass.
