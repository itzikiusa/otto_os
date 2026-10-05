# Iteration 4 — design partition 4

**Provisional source score: 8.2/10. Fixes and rendered acceptance remain required.**

Reviewed workflows, loops, swarm, scheduled tasks, Mission Control and MCP on baseline `03f2bc3e`, observed HEAD `c5d4e999` in `/Users/itziklavon/claude_ade-review`. HEAD adds review documentation; concurrent role-1 edits were left untouched. This reviewer ran no tests, builds, servers, browser interactions or daemon requests. Only this report was written.

Read AGENTS.md, the design README/review checklist/accessibility/layout guidance, PLAN/TRACKER and correctness-4/performance-4/UX-4 reports, Claude's consolidated findings, relevant raw lens reports and coordination note. Compared relevant `b751ca6f` changes for deduplication only: they are external, not this review's source baseline. The original score is retained until an integrated-source rescore. A source score does not establish 9.8 rendered/native acceptance.

## Additional locations and findings

### R4-D4-01 — major: Swarm's org tree exposes a flat set of items without tree keyboard navigation

**Locations:** `ui/src/modules/swarm/OrgTree.svelte:208`, `:230`, `:238`, `:243`, `:281`, `:284`, `:299`.

The parent agent's treeitem ends before its sessions and direct reports render. All descendants become siblings under the outer `role="tree"`; depth is only visual padding. There are no nested groups, `aria-level` attributes, treeitem expansion states or arrow-key handler. Agent treeitems all have `tabindex="-1"` and `aria-selected="false"`; the individual child buttons form the Tab sequence instead. Thus the named tree does not communicate the reporting hierarchy or provide its promised tree navigation. Tab can still reach Edit, disclosure and session buttons: this is not a claim that all actions are unavailable. The Agent Editor's Reports to select (`AgentEditor.svelte:134`) also provides a keyboard reparent path, so drag-only reparenting is not a separate finding.

**Fix:** Implement a coherent tree with nested groups or explicit levels/positions, a roving active item, Up/Down/Home/End navigation, Left/Right expansion/parent-child navigation, and expansion on the treeitem. Keep secondary Edit/Add/Actions controls accessible and retain focus sensibly when collapsing the active descendant. Alternatively deliberately present a nested list with explicit manager labels rather than advertising an unsupported tree.

**Acceptance, unexecuted:** Seed a three-level org with two sibling agents and nested sessions. Verify keyboard navigation and collapse, then inspect the accessibility tree/VoiceOver for correct levels, parent relationships, expanded states and selected session. Editing and changing manager must still work. This extends the tree-widget pattern already identified in partition 3; count it once in an app-wide systemic roll-up. `b751ca6f` does not change OrgTree.

### R4-D4-02 — major: the graph's session chooser is hover-only and can extend beyond its viewport

**Locations:** `ui/src/modules/swarm/AgentGraph.svelte:183`, `:328`, `:347`, `:352`, `:554`.

With an agent holding two or more active sessions, keyboard activation of its graph node or Team brief row always opens `sess[0]`. The only chooser for its other sessions in this graph is conditionally mounted when `hoverAgentId` is set by `mouseenter`. Focusing or activating the node never opens it. Its 210 px absolute popup is positioned below the node with no viewport clamp, height cap or scrolling, while every session is rendered. A graph node near a canvas boundary or a sufficiently long list therefore has no source guarantee that all choices remain visible/reachable. Keyboard users can switch to Org view as a workaround; this does not make the Graph chooser operable. No particular touch browser's synthesized-hover behavior is asserted.

**Fix:** Expose a real Sessions button/menu on each agent, with focus/keyboard activation and selected-agent attribution. Use the shared clamped, scrollable menu or equivalent accessible popover; preserve direct-open convenience if desired. Escape must dismiss and restore focus. Merely adding a CSS hover fallback cannot fix a chooser that is not mounted.

**Acceptance, unexecuted:** Open the chooser using only keyboard and using touch, select the second session, and assert the correct session opens. Seed enough sessions to overflow a phone-height viewport and position the node near each boundary; every entry must remain scrollable, with `expectFullyInViewport` passing. Test focus after dismissal. This is an additional instance of Claude's external hover-only-control pattern (`06-responsive-rtl.md`), counted once here; the popup bounds are included in that one repair, not separately deducted. `b751ca6f` changes only tracking CSS in this file.

### R4-D4-03 — minor: older loop plans have neither pending feedback nor an explicit Retry

**Locations:** `ui/src/modules/loops/IterationRow.svelte:36`, `:44`, `:95`, `:98`; supporting fetch/cache at `ui/src/lib/stores/loops.svelte.ts:123`.

Expanding an older summary starts `loadIteration`, but there is no local pending state. Until it resolves, the plan/context sections simply do not exist while Agents and roles renders as normal. On rejection, a raw-detail status paragraph appears without Retry. The nearby Retry buttons restart blocked executors, not this failed read. Collapsing and expanding can re-trigger the read, but that recovery is undisclosed. This breaks the four-state/inline-retry rule and makes an unloaded plan resemble an absent plan.

**Fix:** Give this read a pending/error state with compact LoadState, a named Retry for the plan/context fetch, and cancellation tied to the same iteration. Retain the already available agent/evaluation summary. Keep read retry distinct from executor restart and distinguish an empty successful body from an unresolved body.

**Acceptance, unexecuted:** Hold an older iteration GET and verify loading feedback; fail it and verify Retry; retry successfully without collapsing and assert plan/context appears. No executor restart request may occur. Collapse during loading and switch loop to check ownership. This extends the existing load-state/retry pattern, charged once rather than charging pending and retry separately. `b751ca6f` only changes heading tracking in this file.

### R4-D4-04 — minor: MCP audit success is absent from the accessible result text

**Locations:** `ui/src/modules/mcp/AuditTab.svelte:202`; `ui/src/lib/components/Icon.svelte:202`.

The OK result cell contains only an Icon for success; Icon always sets `aria-hidden="true"`. Failure has an icon inside a titled span, but success has neither text nor a named result. The surrounding audit grid is made of divs/spans, so the separate OK header does not label each result either. An allowed tool call can succeed or fail: the Decision pill cannot substitute for its execution outcome. This is semantic information loss, not a measured contrast issue.

**Fix:** Add explicit Succeeded/Failed result text (visible, or screen-reader text alongside the glyph), with the relevant failure details reachable without hover. Preserve the distinction between governance decision and execution result. Prefer real table semantics or named row/cell content where the responsive design allows it.

**Acceptance, unexecuted:** Seed allowed-success and allowed-failure rows and assert their accessible text includes distinct outcomes. Inspect with VoiceOver and keyboard; each error must be readable without hovering. `b751ca6f` only changes header tracking in this file.

### R4-D4-05 — major systemic pattern: extend the existing MCP stacked-label repair to Audit

**Locations:** `ui/src/modules/mcp/AuditTab.svelte:195`, `:392`; existing canonical instance `ui/src/modules/mcp/StatsTab.svelte:70`, `:177`.

Audit hides its header at 640 px and turns each row into two columns, but provides no per-cell labels. Server, tool, timestamp, decision, direction, result, latency and bytes become a stream of values without field attribution; long server/tool labels can truncate. Claude's existing major finding covers Stats at 1024 px, where counts and timing/byte values likewise lose their labels. These are one responsive-information pattern, not two major deductions. The distinct missing accessible outcome in R4-D4-04 persists even at desktop and requires its own semantic repair.

**Fix:** Extend the labelled-cell/card pattern to Audit as well as Stats. Keep full server/tool identity discoverable on touch. Do not merely show the hidden header above rows whose column arrangement no longer matches it.

**Acceptance, unexecuted:** In light/dark phone and tablet layouts, verify every displayed value can be matched to its field, including equal-looking values and long names; assert internal scrolling/no page overflow and readable outcome semantics. The `b751ca6f` diff adds labels to Stats, but leaves Audit's responsive markup unchanged. Forward Audit as a missed location within Claude's existing repair.

## Existing external dependencies retained once

These are source-confirmed baseline findings owned by Claude. They are not new Codex implementation tasks. The external catalogue is broader than this focused pass; the table is not a claim that every other external finding is cleared.

| Pattern | Baseline severity / deduction | Source evidence, repair and acceptance |
|---|---|---|
| Workflow canvas lacks keyboard connection/edit operations | major / 0.3 | `workflows/WorkflowCanvas.svelte:259` edge hit path is outside sequential focus; output connection starts via pointer at `:332`. External `05-accessibility.md` finding 1. Integrate accessible edge selection/removal, Connect to and node movement; execute a complete keyboard build/edit journey. Related AgentGraph pan limitations stay in this pattern, not another deduction. |
| Workflow frame bypasses shared page/split structure | major / 0.3 | `workflows/WorkflowsPage.svelte:1757`, `:1759`, `:1847` use direct `.wf`, custom width and split composition instead of PageBody/PaneDivider. External `02-layout.md` major. Integrate shared frame and migrated width preference; inspect header/body proportions, keyboard resizing and phone navigation. The external diff contains the migration; it is not integrated acceptance. |
| Approval presentation is inconsistent about attribution/outcome | minor / 0.1 | `workflows/WorkflowsPage.svelte:1984` omits the available decider from ApprovalOutcome; `mission-control/WorkItemDetail.svelte:374` routes decided states through no-op ApprovalActions and maps non-approved statuses to denied. External agent-pattern findings. Count the approval presentation pattern once. Use explicit outcome/decider semantics; inspect approved, denied, expired and unknown cases. |

External URL-selection and command discovery work remains tracked with Claude and was not refiled here. Scheduled draft loss (UX4), workflow publication and retiming (C4), version payload/cache retention (P4), and swarm bulk-result feedback are excluded from this report's deductions. No source reservations are needed from Codex for these design repairs; root must coordinate the exact files with Claude before implementation.

## Fixed score and coverage

**10 − (5 majors × 0.3) − (3 minors × 0.1) = 8.2/10.** Majors: tree model, hover-only chooser, MCP stacked labels, workflow canvas keyboard, workflow frame. Minors: loop body states, audit outcome semantics, approval presentation. No blockers or nits added. A repeated pattern is charged once, including the external instances above; app-wide aggregation must also deduplicate the partition-3 tree pattern. The rubric is severity subtraction, not a sum of dimension scores.

| Design dimension | Source coverage and strengths | Remaining acceptance |
|---|---|---|
| Shared visual hierarchy | Workflow editor/run/versions/approval regions; Mission Control list/graph/detail; Swarm org/graph/session/board entry points; Loop list/detail/form; scheduled list/edit/history; MCP section tabs/audit/approvals/stats. Most use shared header/body, segmented navigation and inline status vocabulary. | Actual density, pane budgets, long titles, stacking and modal/drawer focus in the rendered app. |
| Tokens/consistency/readability | Sampled semantic status/danger/text tokens, shared Icon/StatusBadge/StatusDot; readable labels and shared loading/error primitives in main loop/MCP lists. No numerical contrast claim made. | All schemes/custom accents, computed text/control contrast, two zoom steps, actual typography and status readability. |
| Responsive layout | Inspected graph containers/popups, MCP stacking CSS and main list/detail phone branches. Explicit per-value labels and popup bounds remain findings. | 390 px phone, tablet and desktop; RTL, overflow, touch access and large-data chooser checks in light/dark. |
| Keyboard/accessibility | Traced org tree vs graph chooser, workflow edges, MCP audit semantics, named Mission Control graph nodes with Enter/Space and selected state, route-backed MCP tablist, loop disclosure expanded state. | Full keyboard journeys, accessibility-tree assertions, VoiceOver, shared modal return focus and native shortcut interception. |
| Inspected rendered states | **None executed by this reviewer.** Existing app-20261004 verification remains credited as prior evidence; it does not certify the newly described fixtures. Claude's reported external UI gates/E2E are not silently imported as this pass's execution. | Pending, empty, stale, failure, retry, long-content and selected states; current integrated revision light/dark captures; packaged Tauri/WKWebView acceptance. |

No score is assigned to an unexecuted rendered dimension. Source findings are directly traced; popup clipping magnitude and assistive-technology presentation require rendering. Scope does not include every workflow node configuration, full external-provider execution, native interactions or all six modules' exhaustive state permutations.

**Release:** Review complete; report only. Additional locations were sent to root for coordination with Claude. No builds/tests/servers/source edits/commits. Read-shell slot released; runtime/native acceptance remains open.
