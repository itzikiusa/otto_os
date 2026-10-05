## Lens: Shared component usage & quality — iteration-5 re-score

**Score: 9.37/10** (10.0 − 2 majors × 0.3 [findings 6 and 7, both Partial] − 1 nit × 0.03 [Modal naming regressed] = 9.37).

Judgement: the arithmetic stands, but I'd put it nearer 9.6. Findings 6 and 7 are down to 5 and about 8 residual local rules, and every other item is verified fixed. The rubric charges a Partial at full original severity, so I did not round.

I read the code in `/Users/itziklavon/claude_ade-design5`; no builds or tests were run.

### Findings

| # | Finding (from the iteration-4 re-score) | Status | Evidence |
|---|---|---|---|
| 2 | Hand-rolled empty states | Fixed | The `RunAgents`, `ProductPage` and old `.empty-state` / `.list-empty` classes no longer match. `database/DbAssistantPanel.svelte:156-159` and `product/MockupAssistPanel.svelte:140-145` keep a layout wrapper (`.da-empty`, `.ma-empty`) around the shared `EmptyState`. |
| 3 | Hand-rolled error blocks | Fixed | A grep for `load-error`, `page-load-error`, `inline-error` and `load-err` class usages in `ui/src` returns no matches. |
| 5 | Hand-rolled switches | Fixed | `TriggersPanel.svelte:361`, `SwarmSettings.svelte:312` and `Inspector.svelte:308,551,561` now use `<Switch>`. `Inspector.svelte:542` is a deliberate `pill-toggle` with `aria-pressed`. |
| 6 | Local spinners and forked keyframes | Partial | The 28 local spinner rules are down to 5: `share/SharePage.svelte:541`, and `agents/conversation/ToolStep.svelte`, `SubagentCard.svelte`, `WorkSteps.svelte` and `LiveStatus.svelte` (one rule each). The forked keyframes are gone: no `@keyframes spin` or `req-tab-spin` remains. |
| 7 | Local pill, chip and badge systems | Partial | Local rule blocks fell from about 70 to 8, in `vault/StructuredNote`, `vault/TagsPanel`, `desktop/TrayPage:376`, `kubernetes/WorkloadPods:222`, `kubernetes/ResourceTable:229-273`, `swarm/SwarmPage:1010` and `swarm/OrgTree:543`. Most of what remains is health or status pills that should route through `Badge`. |
| 8 | Nested interactive in `MonitorOverview` | Fixed | No `role="button"` is left under `ui/src/modules/kubernetes` except a selector string at `ClusterWorkspace.svelte:382`. |
| 10 | Glyph icons | Fixed | A grep for `>(↑|↓|+|−)</button>` returns nothing. `RecoveryTools`, `WorkflowCanvas` and `Terminal` are converted. |
| 11 | Global classes redefined | Fixed | Only modifier variants remain (`.icon-btn.on`, `.icon-btn.small`, `.icon-btn.tool`, `.input.sm`, `.input.file`). No `.btn`, `.btn.danger`, `.btn.warn` or base `.input` overrides are left. |
| 12 | `.fld` form fields | Fixed | `class="fld"` has 0 matches. |
| 13 | Bare "Loading…" text | Fixed | `Loading…` no longer appears in `shell/Navigator.svelte`. |
| 15 | Docked drawer duplication | Fixed | `lib/components/DockedDrawer.svelte` is new. `aws/AwsDrawer.svelte:34` and `kubernetes/ResourceDrawer.svelte:272` use it with no local `pushModal`/`dialogFocus`. On phone it wraps `shell/Drawer` with the `head` snippet and a full-width right sheet. |
| 17 | `EmptyState` drops the CTA silently | Fixed | `lib/components/EmptyState.svelte:41-43` warns in dev when `actionLabel` is set without `onaction`. |
| New (iteration 4) | `MissionControlPage` `onViewKey` fork | Fixed | `function onViewKey` no longer exists anywhere in `ui/src`. |

Spot-check of items I marked Fixed earlier:

- 1, 4, 9, 14, 16 and 18 hold. `Couldn't` has 0 matches in `ui/src`.
- 19 regressed (see below).

### New findings

- **[nit] `Modal` is named by `aria-label` again.** `lib/components/Modal.svelte:157` has `aria-label={title}`, with no `aria-labelledby` or `titleId` left. Iteration 4 had this fixed (`aria-labelledby={titleId}` at the `<h2>`). Restore `aria-labelledby` pointing at the `<h2>` id. This is the same nit as old finding 19, re-opened by a regression, so I counted it as new.

### Remaining to reach 9.8

- Delete the 5 remaining local spinner rules (`share/SharePage.svelte:541` and the four in `agents/conversation/`) in favor of `.spinner`.
- Move the remaining status and health pills to `Badge`: `kubernetes/ResourceTable.svelte:229-273`, `kubernetes/WorkloadPods.svelte:222`, `swarm/SwarmPage.svelte:1010`, `swarm/OrgTree.svelte:543`, `desktop/TrayPage.svelte:376`. Then add a ui-guards ratchet that blocks new `.pill`/`.chip`/`.badge` rule blocks.
- Restore `aria-labelledby` on `Modal.svelte:157`, and add a unit test or guard so it can't silently revert.
