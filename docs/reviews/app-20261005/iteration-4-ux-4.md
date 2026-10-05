# Iteration 4 — UX partition 4

**Verdict: fixes required. Two major findings. Provisional score: 7.6/10.**

Reviewed baseline `03f2bc3e`, observed HEAD `c5d4e999` in `/Users/itziklavon/claude_ade-review`. HEAD adds review documentation; concurrent role-1 changes were left untouched. This is a bounded source review of workflows, loops, swarm, scheduled tasks, Mission Control and MCP. No builds, tests, servers, daemon requests or browser interactions were executed by this reviewer. Only this report was written.

Read AGENTS.md, PLAN/SCORES/TRACKER, the iteration-4 correctness-4 and performance-4 reports, prior partition-4 UX findings and final app-20261004 VERIFICATION, and Claude's consolidated findings/coordination notes. Existing fixed baseline journeys remain credited. Source-confirmed defects below are adjacent behaviors, not re-opened historical findings.

## R4-UX4-01 — major: module navigation silently loses Scheduled Task and Goal Loop drafts

**Locations:** `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:201`; `ui/src/modules/loops/GoalDefineForm.svelte:142`. Navigation mechanism: `ui/src/lib/router.svelte.ts:303`; page replacement: `ui/src/shell/App.svelte:1095`.

**Concrete trace / reproduction:** Open Scheduled Tasks, edit a saved task, and change its name or prompt. The page's Back/Cancel calls `closeForm`, which compares `formState()` to `formSnapshot` and asks before discarding. Instead, select another module in the sidebar. The component has no `router.guard` or `guardUnsaved` registration, so navigation bypasses `closeForm`; the dynamic `<Page />` replacement destroys all local form state. Return and reopen the task: the unsaved changes are gone, with no Keep editing decision or recoverable draft. Repeat in Goal Loops after typing a goal or generating and editing its acceptance criteria: its local `cancel()` asks before discarding, but module navigation never calls it and loses the same work. GoalDefineForm is conditionally mounted by `LoopsPage.svelte:55` and keeps the draft only in local state.

**Expected / actual:** Leaving through the sidebar should give the same protection as the form's Back action. Actual protection depends on which navigation control the user chooses. This is user-authored draft loss on ordinary navigation, not a missing cosmetic warning.

**Fix direction:** Register one reusable dirty/leave decision for the lifetime of each form, reusing the current Scheduled Task snapshot and a complete Goal Loop draft predicate. Route Back/Cancel and module navigation through that decision without double prompting. Keep editing must retain route and every field; explicit Discard may leave. Protect pending save/launch ownership so a departed form cannot later redirect the current view. Keep Claude's URL-selection and presentation work intact. Workspace changes invoke the same router guards; preserve or explicitly settle draft ownership there rather than allowing an old draft to silently acquire a new workspace.

**Required regression, not executed:** In a rendered test, edit Scheduled Task prompt and cadence, navigate via sidebar, choose Keep editing, and assert unchanged route/fields and zero mutation requests. Discard must leave; returning must show the persisted version. Repeat with a Goal Loop draft containing edited acceptance criteria and budget. Check clean navigation does not prompt and browser back/workspace switching use the same guard. Test pending save/launch separately so a late response cannot affect a replacement view. The prior workflow leave test does not cover either of these forms.

## R4-UX4-02 — major: Scheduled Task save discards edits typed while the request is pending

**Locations:** `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte:404`, `:429`, `:432`; editable field example `:707`, form `:692`; only Back/Cancel/Save controls are disabled by `busy` (`:678`, `:914`, `:915`).

**Concrete trace / reproduction:** Edit an existing Scheduled Task, set its name to A, and click Save. Hold the PATCH response. `save()` captures A in `body` and awaits `scheduledTasks.update`. The name input remains enabled; type B, or change the prompt/cadence, while the request is pending. Resolve PATCH and its subsequent list refresh. The success branch unconditionally sets `creating = false` and `editId = null`, removing the form. The server saved A, while the now-lost visible draft was B. Reopening proves B was never saved. The same branch loses later edits during task creation.

**Expected / actual:** A successful save acknowledges its submitted snapshot. It must not silently discard later edits that the interface allowed. Actual behavior announces success and closes the editor while losing those edits. This trace requires ordinary response latency, not concurrent clients or backend failure.

**Fix direction:** Capture form ownership and submitted draft state. On success update the saved baseline, closing only if the active draft still matches the submitted snapshot; otherwise keep the newer fields open and dirty. After create succeeds, retain the returned task ID so saving those later edits updates the created task instead of creating a duplicate. Alternatively consistently disable the entire editable form during save, if that is the chosen product behavior. Fence late completion after leave/workspace change. Do not tie this repair to backend one-shot settlement or workflow version transactions; those are separate existing findings.

**Required regression, not executed:** Delay the existing-task PATCH, change name and prompt after submission, resolve it, and assert newer inputs remain unsaved and available for a second Save (or assert they were consistently disabled). The second Save must persist B to the same ID. Repeat with create: resolve the first POST, retain later edits, then Save must issue PATCH against its returned ID, never a second POST. A rejected save must retain all current inputs and allow retry. Include the intervening list-refresh delay because the current store awaits it before closing the form.

## Checked journeys and deduplication

| Surface | Source paths examined / retained behavior | Execution limits |
|---|---|---|
| Workflows | Save/run entry points, history load/restore and dirty guard. Restore checks current workflow and view generation; version errors have Retry. | C4 graph/version publication and P4 full history/cache findings already own those defects. No new finding or duplicate severity is added here. |
| Goal Loops | Define/refine draft, launch, Back, detail lifecycle, budget extension/resume. Existing partial-resume message now states that budget saved and resume failed. | New form-navigation finding above. Provider execution and real pause/resume were not exercised. |
| Swarm | Agent and goal editors, save failure retention, goal numeric request builder. | Bulk partial failures are Claude's `allSettled` reservation. Modal completion concurrency and complete recruitment/execution remain unexecuted, without an additional finding asserted. |
| Scheduled Tasks | Create/edit/preset, validation, delivery confirmation, save, Back/Cancel, Run now and convert-to-workflow. Delivery names destination and audience; conversion explicitly offers pausing to avoid duplicate schedules. | Two new draft findings. Existing C4 schedule-retiming defects remain canonical. Delivery providers/report execution not exercised. |
| Mission Control | Detail edit/load/approval paths, owner/view guards, dirty leave decision and preservation of edits typed during Save. | Prior selection-loss repair remains closed. Source contains a reusable example of submitted-draft comparison. No fresh runtime item/approval test. |
| MCP | Approval loading/filter generation, inline Retry, decision failure, always-allow entry, token create/rotate/revoke and copy failure. | Claude owns approval presentation and token owner-list failure feedback. No new governance defect claimed; actual tool/credential use not attempted. |

Reservations required: `ui/src/modules/scheduled-tasks/ScheduledTasksPage.svelte` form leave/save ownership only; `ui/src/modules/loops/GoalDefineForm.svelte` draft leave protection only, plus focused regression files. Claude owns URL selection, visual/a11y/copy. No source edits were made or reserved by this report itself; needs were sent to root for coordination.

## Fixed rubric and evidence

| UX dimension | /2 | Evidence and deduction |
|---|---:|---|
| Task completion/discovery | 1.7 | Core list/create/run/approval paths are present; completing form edits can lose later work, and complete cross-module execution remains unverified. |
| Feedback/state clarity | 1.8 | Inline loaders/retry and explicit partial-success resume feedback exist. Scheduled Save success does not distinguish lost post-submit edits. |
| Recovery/retry | 1.7 | Failed saves retain fields and sampled loaders offer Retry; discarded form drafts have no recovery. |
| Draft/scope/trust preservation | 1.3 | Destination confirmations and Mission Control ownership guards are concrete strengths; two confirmed draft-loss mechanisms materially weaken this dimension. |
| Executed end-to-end journeys | 1.1 | Verified green baseline plus one explicitly mapped workflow leave journey; no named execution for new forms' failure paths or the full six-surface matrix. The extra 0.1 over unmapped green baseline credits only that named journey. |
| **Total** | **7.6/10** | **Source-only provisional assessment. Major findings prevent 9.8 acceptance regardless of arithmetic.** |

Prior execution is preserved from `../app-20261004/VERIFICATION.md:46`: rendered workflow leave regression passed (Cancel preserves draft, failed Save stays, successful Save persists before leaving, Discard leaves). Final integration at `:96` onward records 1,081 UI unit passes and 98 distinct affected browser cases across the combined run and repaired reruns; it does not map all those cases to this partition. The full desktop suite was not uniformly green. This review neither erases that evidence nor claims those runs certify the new defects. Root should append current-revision regression/browser results and any calibrated rescore without replacing this initial assessment. Native Tauri, assistive technology and outward-provider acceptance remain unexecuted here.
