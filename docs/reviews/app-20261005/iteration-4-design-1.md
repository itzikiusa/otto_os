# Iteration 4 design review — partition 1

**Provisional bounded source score: 8.8/10. Not accepted for 9.8.** Baseline `03f2bc3e380cbee6da22723872ba865a84aa8050`, branch `fix/app-review-20261005`. This pass reviewed shell/navigation, session tabs, conversation previews, History, terminal presentation and adjacent keyboard paths. It found two additional minor patterns and retained three major and one minor externally owned dependencies. No browser, native app, tests, builds or servers were run. Only this report was written.

## Method and ownership

Read `AGENTS.md`, `PLAN.md`, design guidelines README/accessibility/layout/components/review-checklist, coordination note `/tmp/otto-app-review-coordination-20261005.txt`, consolidated Claude findings and relevant raw reports under `/tmp/otto-design-iter4-reports/`. This is a bounded independent review of actual source, not a whole-app visual certification or a replacement for Claude's ten-lens report.

Claude implementer A owns the two proposed presentation/keyboard repairs below. The existing History active-session restart and view-cache findings remain correctness1-owned. New-session partial batch recovery and loaded-history search disclosure remain UX1-owned. Neither is duplicated or deducted here. No source paths were edited or reserved by this reviewer.

## Additional findings

### R4-D1-01 — minor — session tabs lack complete keyboard equivalents

**Locations:** `ui/src/shell/TabBar.svelte:88`, `:89`, `:203`, `:207`, `:246`. Related existing semantic-pattern extension: `:245` and `:348`.

**Source trace / user reproduction:** Open three sessions, set the app to RTL, focus the middle tab, and press Right. The flex tab row inherits RTL, but the handler always chooses array index +1; it never checks computed direction, so it moves toward the visually left-hand tab. Separately, try to reorder sessions without a pointer: the only `ws.reorderTab` call in `ui/src` is `onDrop` at line 207. The tab menu and key handler expose no move operation. Switching sessions with arrows or global shortcuts does not reorder the persisted tabs. Every tab also has `tabindex="0"`, so Tab walks all tab controls rather than leaving a single active tab stop.

**Rule:** `docs/design/guidelines/accessibility.md` §3 requires keyboard equivalents for pointer operations and composite tab keys; §6 requires RTL behavior. This is secondary interaction friction, not a blocker to reading or running a session. Count this incomplete keyboard pattern once.

**Fix direction:** Make arrow navigation direction-aware, retain Home/End, use roving tabindex, and expose move-before/move-after actions through an accessible menu or documented key binding. Keep the close button and rename field outside the tab's interactive semantics. The latter extends Claude's existing `05-accessibility.md` finding 3 (Database tabs containing controls); do not deduct that systemic issue again for this new location.

**Verification idea, not executed:** In LTR and RTL, focus the middle of three tabs and assert the next focused/selected tab follows physical arrow direction. Verify Tab exits the tablist from the active tab. Invoke the new reorder alternative without dragging and assert the persisted order, including first/last disabled boundaries. Check close and rename keyboard behavior after restructuring; inspect the accessibility tree for nested tab controls.

### R4-D1-02 — minor — expanded file-preview errors have no Retry

**Locations:** `ui/src/modules/agents/conversation/PreviewBody.svelte:174`–`:177`; `ui/src/modules/agents/conversation/PreviewPanel.svelte:167`–`:177`. Error population: `PreviewBody.svelte:131`–`:134`; existing reload capability: `PreviewPanel.svelte:66`–`:68`, `:148`–`:149`.

**Source trace / user reproduction:** Open a transcript file reference whose content must be loaded from disk, expand it with Full view while the read is pending, and fail the read. The full modal renders `PreviewBody`, whose error branch shows a heading and raw detail through `EmptyState` with no action. The reload icon exists only in the underlying side-panel toolbar; the modal contains just the view tabs and body. A user must close the expanded view, discover the side-panel reload icon, then reopen the expanded view to recover. The modal's own error cannot be retried in place.

**Rule:** `docs/design/guidelines/components.md` §11 and AGENTS.md require failed loads to show inline recovery with Retry, preferably `LoadState`. This is a recoverability/design-state omission; a recovery path exists outside the modal, so minor is appropriate.

**Fix direction:** Give the common preview error region an explicit retry callback that clears the relevant cached read and advances reload state, and pass it from both preview hosts. Use the shared error-state presentation with a human cause. Keep non-retryable unsupported-image guidance distinct from a transient read failure.

**Verification idea, not executed:** Delay and reject `/fs/read`, open Full view before rejection, assert an inline Retry is available in the active dialog, recover the route, invoke Retry, and assert the preview loads without closing the dialog. Repeat for an image artifact network failure and verify permanent unsupported-image guidance remains accurate.

## Existing dependencies, deduplicated

These remain present in this worktree's baseline source. They are already assigned to Claude A; no parallel repair is requested. Ownership or a fix in another worktree is not evidence that the baseline is repaired.

| Dependency | Severity / deduction | Source and existing owner evidence | User scenario / rule / acceptance idea |
|---|---:|---|---|
| Force-dark terminal wrapper does not establish dark overlay tokens | major / 0.3 | `ui/src/lib/components/Terminal.svelte:3046`, `:3064`; Claude `09-theming-motion.md` finding 1 | Light app scheme with force-dark terminal plus Find/overlay; shared surface/token consistency. Integrate scoped dark tokens and inspect light/dark find, badges and toolbar. No rendered contrast claim here. |
| Inactive tab close controls remain hover-dependent on touch | major / 0.3 | `ui/src/shell/TabBar.svelte:531`–`:549`; Claude `06-responsive-rtl.md` hover-reveal finding | Touch/coarse pointer with inactive tabs: opacity remains zero absent hover/focus. Shared touch-accessibility rule. Integrate reveal utility and verify inactive-tab close discoverability on a touch viewport. |
| Conversation claims active work during disconnected events | major / 0.3 | `ui/src/modules/agents/conversation/LiveStatus.svelte:44`–`:50`, `ConversationView.svelte:843`–`:844`; Claude `08-agent-patterns.md` finding 1 | Drop event socket during work: live line still pulses/ticks while composer reconnects. Stale-is-a-state rule. Integrate stale prop/state and assert reconnect copy with no working spinner/elapsed clock. |
| Session tab close hit area too small for coarse pointer | minor / 0.1 | `ui/src/shell/TabBar.svelte:534`–`:535`; Claude `06-responsive-rtl.md` small-tab-controls finding | Close target is 16×16 and lacks its own coarse-pointer expansion. Responsive hit-area rule. Verify at least the required 36px hit area without overlap after integration. |

The full external catalogue also contains shared token/copy/spacing/navigation work. This focused score does not purport to re-audit or rescore that entire catalogue. Root should keep Claude's full design scores and this partition score separately visible.

## Score and coverage

Fixed design deduction rubric: **10 − (3 × 0.3 major) − (3 × 0.1 minor) = 8.8**. The minor count comprises the two new patterns and coarse-pointer hit size. Zero blockers or nits added. Systemic repetitions, including nested tab semantics, are counted once rather than once per file. External ownership does not remove the known deductions. No credit has been taken for unintegrated or unverified fixes.

| Design dimension | Inspected source evidence | Remaining acceptance |
|---|---|---|
| Shared hierarchy | Agents TabBar exception; History PageHeader/PageBody; preview title/actions; RightPanel tabs/overflow and Notes state | Actual toolbar fit and focus order at desktop/tablet/phone |
| Tokens/consistency/readability | Terminal dark-wrapper and overlay dependency; Lightbox scrim dependency acknowledged; token-sized text and truncation in sampled controls | Computed contrast, all themes/custom accent, long labels and rendered text |
| Responsive layout | History list/detail switch and divider suppression; PreviewPanel covering/inert logic; Composer popup max-height; tab close styling | Screenshots at phone/tablet/desktop, two zoom steps, light/dark, RTL; no runtime overflow assertion made |
| Keyboard/accessibility | TabBar key/menu/drag call paths; PreviewPanel local tabs and focus return; Lightbox dialogFocus; History real row buttons; split/tile separator handlers sampled | Keyboard-only journeys, accessibility tree/axe and VoiceOver; new tab findings remain open |
| Rendered/native states | **None executed in this pass** | Native WKWebView/PTY interaction, screenshots, stale/error/loading rendering, focus restoration across overlays |

Source protections worth retaining: History suppresses auto-selection on narrow viewports and presents one pane at a time; PreviewPanel marks covered chat content inert and restores focus; Lightbox registers with overlay accounting and uses dialogFocus; RightPanel loads chunks with retry and retains browser state when hidden. These are source observations, not passing journey results.

**Release status:** source review complete; no rendered/native acceptance claimed. Two additional minor patterns forwarded to root for Claude A coordination. Existing major dependencies prevent 9.8 acceptance regardless of the score after local-only fixes. Read-shell slot released on completion.
