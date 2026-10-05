## Lens: Agent-facing and trust patterns (patterns.md)

**Score: 7.7/10.** Arithmetic: 10.0 − 0.6 (2 major) − 1.6 (16 minor) − 0.09 (3 nit) = 7.71. Two real trust gaps hold it down: the conversation's "working" line ignores a dropped events socket, and Product generations (Plan, Rewrite) have no Stop. Below those sit many small inconsistencies in Stop, proposal and agent-label treatment.

**Verified from the earlier passes (credit)**
- `ApprovalActions` and `ApprovalOutcome` are wired into 9 surfaces:
  - MCP approvals
  - Work item gates
  - Run with Otto
  - Workflows
  - Findings
  - Self-improvement
  - Browser approval card
  - Assistant `NeedsYouCard`
  - The Deny verb is neutral, and the busy labels name the action.
- `confirmOutward` is used across the Git, Product, Scheduled, Channels, Share and `uiCommands` surfaces. It always states Where, What and Who sees it.
- `confirmProd` covers Kubernetes (typed-name confirm plus a prod gate on every mutating action), AWS, Brokers and SFTP.
- The Design Hall variants tray (Apply, Compare, Reject with reasons) and the "Apply on top" confirm follow §2 and §3.
- `AgentChip` and `AgentByline` are used in the conversation, Refine chat, Board feed and the Run-with-Otto gate.
- `sessionState(..., { stale })` is passed from 17 call sites.
- Join requests correctly use Admit and Decline.

### Findings

**Major**

1. **[major] The conversation's "is working" line ignores a dropped events socket**
   - Locations: `ui/src/modules/agents/conversation/LiveStatus.svelte:44-50`, `ConversationView.svelte:180-183, 307, 843-844`.
   - `LiveStatus` always shows a spinner, "X is working" and a ticking elapsed clock whenever `status === 'working'`. It never reads `events.state`, although `Composer.svelte:90-94` in the same pane does turn the state into "Reconnecting…".
   - This breaks §1 "Stale is a state": the pane shows both a stale composer and a live-looking thread.
   - Fix: pass `stale` (or the `sessionState` result) into `LiveStatus`. When stale, drop the spinner and the elapsed time and say "Reconnecting…".

2. **[major] Plan and Rewrite generation have no Stop or Cancel while running**
   - Locations: `ui/src/modules/product/PlanTab.svelte:464-478, 549-550`, `RewriteTab.svelte:235-250`.
   - The only signal is a disabled "Generating…" button and an italic "checking every 3s…". The agents keep running with no way to end them. §1 says "Cancel/Stop is always reachable while something runs", and `AnalysisTab.svelte:547-551` already has a per-agent Stop.
   - Fix: add a neutral `stop`-icon Stop button wired to the same stop path while `generating || pollTimer !== null`.

**Minor**

3. **[minor] The Stop icon is `square` in 8 places; §7 says one icon, `stop`**
   - Locations: `WorkflowsPage.svelte:1739, 2151`, `skills-eval/RunDetail.svelte:423`, `skills-eval/MatrixView.svelte:422`, `swarm/KanbanBoard.svelte:441`, `skills-lab/SkillReviewPanel.svelte:417`, `loops/LoopDetail.svelte:115`, `swarm/SwarmPage.svelte:454`.
   - `RequestBuilder.svelte:786` uses an `x` icon. `Ec2View.svelte:182` and `kubernetes/actions.ts:53, 64` also use `x` for stop-like verbs.
   - Fix: switch these to `stop`. A guard rule on `name="square"` next to the word Stop would hold the line.

4. **[minor] Stop styling and confirm rules don't follow §7**
   - Neutral and immediate is for ending a query or a turn only. Red, with "…" and a confirm, is for ending an agent session or worktree.
   - Wrong in the "should be neutral" direction:
     - `aws/LogsView.svelte:640` is a red `.btn danger` Stop for a CloudWatch Insights query.
     - `api/AutomationEditor.svelte:214-216, 285` confirms before stopping an API step run.
   - Wrong in the "should be red" direction:
     - `skills-eval/RunDetail.svelte:211-212, 422` and `MatrixView.svelte:135, 421` are neutral buttons without "…", yet they end agent evaluation runs and confirm.
     - `KanbanBoard.svelte:440` ("Stop the planner agents") is neutral with no confirm, no "…" and no red.
   - Fix: align each one to the §7 rule.

5. **[minor] The Workflows stop verb is "Stop run…" on the button but "Cancel run" in the dialog**
   - Locations: `WorkflowsPage.svelte:1069` vs `:1739` and `:2151`.
   - The default Cancel button then sits next to a "Cancel run" confirm. `confirm.svelte.ts:31-33` says to override `cancelLabel` in exactly this case, and `run-with-otto/RunDetail.svelte:65` does ("Keep running").
   - §7 says "the confirm button repeats the verb".
   - Fix: use `confirmLabel: 'Stop run'`, `title: 'Stop run'` and `cancelLabel: 'Keep running'`.

6. **[minor] Emoji avatars still ship for personal agents (§1 "No emoji avatars")**
   - Locations: `personal-agents/templates.ts:25` (`'🛡️'` and the other templates), `AgentEditSheet.svelte:178, 191` (the emoji placeholder and `{t.avatar}` in the option text), `AgentAvatar.svelte:3-4` ("usually an emoji").
   - Fix: make template avatars empty so they fall back to the monogram tile. Drop the emoji placeholder and the emoji in the option label.

7. **[minor] Agent proposals mix vocabulary and button styling (§5 vocabulary table)**
   - Proposals should use Accept/Keep and Reject/Discard. Gates use Approve/Deny.
   - Proposal buttons:
     - `design-hall/LearnedPage.svelte:475-478`: primary "Approve" plus ghost "Reject".
     - `assistant/MemoryTab.svelte:228-229`: ghost "Reject" plus a plain (non-primary) "Keep".
     - `assistant/cards/NeedsYouCard.svelte:199-200`: neutral "Reject" plus primary "Keep".
     - `product/TestCasesTab.svelte:837` and `design-hall/brand/BrandEditor.svelte:486, 561` say "Approve" for agent drafts.
     - `design-hall/assist/ruleActions.ts:39-40` confirms Reject on a rule proposal, while the memory Reject is immediate.
   - Gate wording is muddled in one place: `git/FindingActions.svelte:249, 264` has an overflow-menu "Accept" and a gate that says "Approving accepts it".
   - Fix: one pair per kind. Proposals get Reject (neutral) and Keep or Accept (primary), with the same reject-confirm rule everywhere.

8. **[minor] Several agent labels are hand-rolled instead of `AgentChip`**
   - Locations: `api/HistoryList.svelte:225` (a bespoke `.chip agent`), `browser/live/RemoteLiveView.svelte:848` (a bare `.chip`), `swarm/RunsList.svelte:119` (`c-agent`), `product/RefineChat.svelte:158` and `product/DiscoveryChat.svelte:219, 233` (a plain "Agent" bubble-role while thinking).
   - Fix: use `<AgentChip />` or `AgentByline` in each.

9. **[minor] Hand-rolled "working…" text instead of `LiveWorkingDot`, so stale is not honoured**
   - Locations: `canvas/ConversationPanel.svelte:132`, `product/MockupAssistPanel.svelte:116`. `DbAssistantPanel.svelte:118` and `assistant/ChatView.svelte:202` do it right.
   - Fix: swap in `<LiveWorkingDot label="Working…" />`.

10. **[minor] Approval outcomes omit who decided**
    - `FindingActions.svelte:276-279` passes no `by`.
    - `WorkflowsPage.svelte:1984-1988` doesn't pass `run.approved_by`.
    - §5 wants "Approved by Dana · 3m ago".
    - Fix: pass the name through, resolving the user as `WorkItemDetail` does.

11. **[minor] `WorkItemDetail` fakes an outcome with a no-op `ApprovalActions`**
    - `mission-control/WorkItemDetail.svelte:374-379` passes `onapprove={() => {}}` and `ondeny={() => {}}` just to use `decided`.
    - A decided gate with no `decided_by` (expired or auto-decided) renders nothing at all.
    - Fix: render `<ApprovalOutcome>` directly, and handle `expired` and unknown deciders.

12. **[minor] The workflow `human_approval` banner is thin compared with the §5 queue shape**
    - `WorkflowsPage.svelte:1967-1981` shows the node name and the run start time, but no node output, prompt or agent to approve.
    - Fix: show the agent or step, a what-you-approve summary, and the reason, like `RunDetail.svelte:206-212` does.

13. **[minor] The UI-control grant request is a hand-rolled approval**
    - `agents/SessionView.svelte:944-953` has its own Deny and "Allow for this session" pair with bespoke verbs, instead of `ApprovalActions`.
    - Fix: use `ApprovalActions` with `approveLabel="Allow for this session"`, or document it as a join-style exception.

14. **[minor] Viewer share links (no email) skip the outward confirm**
    - `agents/ShareModal.svelte:111-128` confirms only for editor links or an email recipient. "Sharing a session remotely" is on §5's outward list, and a viewer link lets anyone with the URL read a terminal.
    - Fix: confirm all links, or state why the form is itself the confirmation.

15. **[minor] PR comments post with no confirm, while Jira comments confirm**
    - `git/PrDetail.svelte:274-306, 346-357` posts general and inline comments with no confirm. `product/OverviewTab.svelte:1058` confirms a Jira comment, and `PrDetail` itself confirms approve, decline and request-changes.
    - Fix: either confirm PR comments, or add a short documented exception (comment composers are explicit sends) to §5.

16. **[minor] The S3 presign confirm's buttons name a duration, not the action**
    - `aws/S3Browser.svelte:398-409` offers "1 hour", "12 hours", and so on. §5 says the primary button names the action.
    - Fix: label them "Copy link · 1 hour", and so on.

17. **[minor] Menu rows that open a confirm lack the trailing "…"**
    - Locations: `ProductPage.svelte:156`, `aws/AccountRail.svelte:77`, `git/GraphView.svelte:1211, 1608`, `database/DatabasePage.svelte:241`, `database/WidgetCard.svelte:131`, `swarm/OrgTree.svelte:116`, `brokers/BrokersPage.svelte:205`, `home/HomePage.svelte:106`, `shell/Navigator.svelte:1435, 1538`, `shell/TabBar.svelte:162`.
    - Fix: end them with "…", as `LoopDetail.svelte:115-116` and `Ec2View` do. Rows such as `kubernetes/actions.ts:41` and `:53` ("Delete pod", "Abort") lead to a confirm too.

18. **[minor] `PrDetail` toasts use raw `e.message` and non-Couldn't titles instead of `toastError`**
    - `git/PrDetail.svelte:227` ("Request changes failed"), `:268` ("Update failed"), `:334, 353, 384` (straight apostrophe in `"Couldn't …"`).
    - The same pattern appears in `product/RewriteTab.svelte:198` ("Publish failed"), `AnalysisTab.svelte:320` ("Stop failed") and `FindingActions.svelte:207` ("Could not convert to Jira").
    - Fix: use `toastError('Couldn’t …', e)`.

**Nit**

19. **[nit] Canvas agent edits go straight onto the scene.** `canvas/D2Canvas.svelte:285-292` (the Mermaid canvas is the same) ingests the agent's output without an Apply step. "Restore previous version" in `ConversationPanel.svelte:133-141` mitigates it. Consider a one-line "Otto edited · Undo" affordance.

20. **[nit] The `ApprovalsTab` decided fallback is plain text.** `mcp/ApprovalsTab.svelte:256-260` should route through `ApprovalOutcome` for unknown statuses.

21. **[nit] Deny in Run with Otto destroys a worktree, and it is only mentioned in the gate note.** `run-with-otto/RunDetail.svelte:214-224`. Put "removes its worktree" in the `DenySheet` hint, via `denyTarget` or a `denyHint` prop.

### What would get this lens to 9.8

- Make `LiveStatus`, the canvas and mockup "working…" lines, and every agent "thinking" bubble derive from `sessionState` with `stale`, and use `LiveWorkingDot` and `AgentChip` only.
- Give every long-running generation or run a reachable Stop, starting with Plan and Rewrite. Normalise the `stop` icon, the neutral-versus-red rule and the confirm verb through one `StopButton` or `confirmStop` helper.
- Finish the approval convergence:
  - Move `SessionView`'s UI-control grant and the `WorkItemDetail` fake onto the shared components.
  - Pass `by` everywhere.
  - Give the workflow banner a what and why.
  - Fix the proposal vocabulary and styling (Reject/Keep) so it is consistent across Memory, Learned, `NeedsYouCard`, Test cases and Brand.
- Close the outward-confirm gaps (viewer share links, PR comments) or document them as exceptions in patterns.md §5. Rename the presign buttons to actions.
- Remove the emoji avatar templates and add a guard against emoji in agent avatars.
- Add ui-guards rules for:
  - `Icon name="square"` next to "Stop".
  - Hand-rolled `chip` with the text "Agent".
  - Menu `danger: true` rows without a trailing "…".
