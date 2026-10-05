## Lens: Agent-facing and trust patterns — re-score

**Score: 9.0/10.** Arithmetic: 10.0 − 0.3 (1 Partial major) − 0.6 (6 Open or Partial minors) − 0.06 (2 Open nits) − 0.03 (1 new nit) = 9.01.

### Previous findings

| # | Finding | Status | Evidence |
|---|---|---|---|
| 1 | `LiveStatus` ignores a dropped events socket | Fixed | `LiveStatus.svelte:25-31, 49-52` adds the `stale` prop and a "Reconnecting…" line with no spinner or clock. `ConversationView.svelte:871` passes `{stale}` and `sessionInfo.hint`. |
| 2 | Plan and Rewrite generation have no Stop | Partial | `PlanTab.svelte:485, 563` and `RewriteTab.svelte:258` now have a neutral Stop. It only stops waiting (`stopWaiting`, `PlanTab.svelte:161-165`), and the agents already started keep running, so it does not end the work. |
| 3 | Stop icon is `square` in many places | Partial | Nearly all swapped to `stop` (Workflows `:1876, 2281`, Matrix, Skills RunDetail, Kanban, SkillReview). Still `square`: `SwarmPage.svelte:500` ("Abort all…") and `LoopDetail.svelte:115` ("Stop loop…"). |
| 4 | Stop styling and confirm rules | Fixed | `LogsView.svelte:641` is neutral. `AutomationEditor.svelte:281` is neutral and immediate. `MatrixView.svelte:424` and `skills-eval/RunDetail.svelte:426` read "Stop…". `KanbanBoard.svelte:318-324, 470` is "Stop…" with a danger confirm. |
| 5 | Workflows "Cancel run" confirm vs "Stop run…" button | Fixed | `WorkflowsPage.svelte:1142` uses `confirmLabel 'Stop run'` and `cancelLabel 'Keep running'`. |
| 6 | Emoji avatars in templates | Fixed | `templates.ts:26` is `avatar: ''`. No emoji placeholder or `{t.avatar}` remains in `AgentEditSheet`. |
| 7 | Proposal vocabulary and styling | Fixed | `LearnedPage.svelte:475-476` is Reject (neutral) then Accept (primary). `MemoryTab.svelte:235-236` is Reject then primary Keep. `NeedsYouCard.svelte:203` matches. |
| 8 | Hand-rolled "Agent" labels | Partial | `HistoryList.svelte:229` and `RemoteLiveView.svelte:849` now use `AgentChip`. `RefineChat.svelte:158` and `DiscoveryChat.svelte:219` still use a bare `bubble-role` "Agent" for the thinking bubble. |
| 9 | Hand-rolled "working…" text | Partial | `ConversationPanel.svelte:133` now uses `LiveWorkingDot`. `MockupAssistPanel.svelte:116` is still plain `working…` text. |
| 10 | Approval outcomes omit who decided | Fixed | `FindingActions.svelte:278` passes `by`. `WorkflowsPage.svelte:2115` passes `by={approverLabel(...)}`. |
| 11 | `WorkItemDetail` no-op `ApprovalActions` | Open | `WorkItemDetail.svelte:375-379` is unchanged. |
| 12 | Workflow approval banner is thin | Fixed | `WorkflowsPage.svelte:2095-2101` adds the upstream steps, the ask text and what approving runs next. |
| 13 | UI-control grant not on the shared component | Fixed | `SessionView.svelte:953-961` uses `ApprovalActions` with `approveLabel="Allow for this session"`. |
| 14 | Viewer share links skip the outward confirm | Fixed | `ShareModal.svelte:118-119` now confirms every link ("Create viewer link"). |
| 15 | PR comments have no confirm | Fixed | Documented as an exception in `patterns.md:191` (the composer shows its own "Posts to PR #N…" line). |
| 16 | Presign buttons named by duration | Fixed | `S3Browser.svelte:399-403` reads "Copy link · 1 hour" and so on. |
| 17 | Menu rows that open a confirm lack "…" | Open | Still bare: `TabBar.svelte:239`, `OrgTree.svelte:116`, `Navigator.svelte:1442, 1545`, `BrokersPage.svelte:206`, `AccountRail.svelte:78`, `GraphView.svelte:1223, 1620`, `WidgetCard.svelte:131`, `HomePage.svelte:106`, `ProductPage.svelte:158`, `kubernetes/actions.ts:41, 53`. |
| 18 | `PrDetail` toasts use raw `e.message` | Partial | `toastError` is now used at `:235, 276`, and titles read "Couldn’t …". Still raw `e.message` with non-Couldn't titles at `:342` ("Resolve failed"), `:361, 392, 423`. |
| 19 | Canvas agent edits apply directly | Open (nit) | `D2Canvas.svelte:289` is unchanged. |
| 20 | `ApprovalsTab` decided fallback | Fixed | `ApprovalsTab.svelte:253-264` uses `ApprovalOutcome` and an outcome-line shape. |
| 21 | Run with Otto Deny destroys a worktree | Open (nit) | `run-with-otto/RunDetail.svelte:215` still states it only in the gate note. |

### New findings

- **[nit] The Stop glyph is reused as a shape icon.** `canvas/ToolRail.svelte:76` draws the Shape tool with `<Icon name="stop" />`. §7 reserves `stop` for ending running work. Use `square` there (`ToolRail.svelte:33` already uses it for Image).
- I found no regressions from the new `stale`, `AgentChip` or `ApprovalActions` wiring in the files I re-read.

### Remaining to reach 9.8

- Give Plan and Rewrite a real cancel that stops the agents (or relabel the button "Stop waiting"), and finish the `square` to `stop` swap in `SwarmPage.svelte:500` and `LoopDetail.svelte:115`.
- Use `AgentChip` or `AgentByline` for the Refine and Discovery thinking bubbles, and `LiveWorkingDot` in `MockupAssistPanel.svelte:116`.
- Replace the no-op `ApprovalActions` in `WorkItemDetail.svelte:375` with `ApprovalOutcome`, and handle gates decided without a `decided_by`.
- Add the trailing "…" to the confirm-leading danger menu rows, backed by a ui-guards rule.
- Move the remaining `PrDetail` toasts (`:342, 361, 392, 423`) to `toastError`. Surface the Deny consequence for Run with Otto in the `DenySheet`, and add an "Otto edited · Undo" affordance on the canvas.
