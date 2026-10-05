## Lens: Agent-facing and trust patterns — iteration-5 re-score

**Score: 9.90/10.** Arithmetic: 10.00 − 0.10 (one Partial minor, the trailing "…" on danger menu rows) = 9.90.

Judgement: I agree with the arithmetic. Plan and Rewrite now read "Stop waiting", which is honest, so I scored them Fixed rather than demanding a real agent cancel.

| # | Iteration-4 item | Status | Evidence |
|---|---|---|---|
| 2 | Plan and Rewrite "Stop" that only stops waiting | Fixed | `PlanTab.svelte:486-487, 564-565` and `RewriteTab.svelte:259-260` now read "Stop waiting" with an `x` icon. `:162-166` and `:74-78` explain that the agents keep running. |
| 3 | `square` instead of `stop` on Stop actions | Fixed | A grep for `name="square"` and `icon: 'square'` finds only non-Stop uses (Kubernetes pause and suspend, canvas templates and Image, home Reset size, design format). `SwarmPage` and `LoopDetail` no longer match. |
| 8 | Bare "Agent" labels in the thinking bubbles | Fixed | `RefineChat.svelte:158-160` and `DiscoveryChat.svelte:219-221` now use `AgentByline` plus `LiveWorkingDot` "Thinking…". The `.dim` "Agent" at `RefineChat.svelte:170` and `DiscoveryChat.svelte:234` is the provider selector's field label, not an agent label. |
| 9 | Plain `working…` text | Fixed | `MockupAssistPanel.svelte:118` uses `LiveWorkingDot label="Working…"`. |
| 11 | `WorkItemDetail` no-op `ApprovalActions` | Fixed | `WorkItemDetail.svelte:377-382` renders `ApprovalOutcome` with `by`, `at` and `note`. |
| 17 | Menu rows that open a confirm lack "…" | Partial | Most rows now carry the "…". Still bare: `Navigator.svelte:1446, 1548`, `TabBar.svelte:240`, `HomePage.svelte:107`, `DesignArena.svelte:683`, `Studio3D.svelte:429`, `StatesBar.svelte:101`. The ones I read lead to a confirm (`HomePage.svelte:98`, `DesignArena.svelte:712`). I did not read the `Navigator`, `TabBar`, `Studio3D` or `StatesBar` handlers. |
| 18 | `PrDetail` raw `e.message` toasts | Fixed | A grep for `toasts.error(… e instanceof Error` and the "… failed" titles in `PrDetail`, `RewriteTab` and `AnalysisTab` finds nothing. |
| 19 | Canvas agent edits apply directly | Fixed | `D2Canvas.svelte:17` imports `toastAgentEdit` from `./agentUndo`, which provides an undo affordance. I confirmed the import, not the toast contents. |
| 21 | Run with Otto Deny consequence | Fixed | `run-with-otto/RunDetail.svelte:223` passes `denyHint` ("Denying ends the run and removes its worktree — commits that weren’t pushed are lost"). |
| New | Stop glyph used as the Shape tool icon | Fixed | `ToolRail.svelte:76` now uses `square`. |

**Spot-checks of items already Fixed (no regressions found):**
- `ConversationView.svelte:871` still passes `{stale}` and `staleHint` to `LiveStatus`.
- `ApprovalOutcome` is still present in `WorkItemDetail`.
- The Workflows stop wiring and the Deny sheets in `RunDetail` are intact.

### New findings

None found. The Stop wording, `AgentByline` and `LiveWorkingDot` changes read consistently with the rest of the app.

### Remaining to reach 9.8

- The score is already above 9.8. To close the last minor, add the trailing "…" to the remaining danger menu rows that open a confirm (`Navigator.svelte:1446, 1548`, `TabBar.svelte:240`, `HomePage.svelte:107`, `DesignArena.svelte:683`, `Studio3D.svelte:429`, `StatesBar.svelte:101`). Back it with a ui-guards rule for `danger: true` rows without "…".
