# Iteration 1 — visual design partition 5

**Verdict: Fix the uncovered accessibility gaps.** One major and two minor findings. All are source-confirmed; no runtime, screenshot, contrast measurement, screen-reader session, or responsive browser run was performed.

Reviewed the current review worktree (HEAD `a16f4c71d5b158359fc3507c9f883c500c1f88a3`) using `AGENTS.md`, design guidelines README, review checklist, and accessibility guidance. Read correctness/performance partition 5 and the coordination file; searched existing application-review reports for duplicates. Compared the relevant files against the separate Claude design worktree's working diff from `main`. Claude owns all fixes below. Only this report was written; no builds, tests, servers, source edits, or external actions.

## D5-01 — Major — Room Point and Highlight annotations cannot be created with a keyboard

**Location:** `ui/src/modules/rooms/RoomAnnotations.svelte:34`; creation handlers at `:16` and `:26`, tool buttons at `:43`. Mounted on an active shared stream by `ui/src/modules/rooms/RoomScreens.svelte:28`.

**Scenario and evidence:** An admitted participant receives permission to annotate a shared screen. Keyboard navigation reaches the Point/Highlight buttons and changes the selected tool, but cannot create a mark. The annotation SVG has `role="img"`, no focus target or key handler, and only pointer-down/move/up handlers populate and submit `points`. There is no alternate coordinate or placement control. Selecting a tool does not itself place anything. This finding concerns point placement and rectangular highlighting; it does not assume that freehand drawing must reproduce every pointer gesture by keyboard.

**Impact:** A keyboard-only participant can request permission and choose a tool but cannot perform the annotation action offered by those controls.

**Smallest fix:** Provide a focusable placement surface or a labelled keyboard placement control for Point and Highlight. Show a visible cursor/rectangle, allow arrow-key adjustment, Enter to place/complete and Escape to cancel, and reuse the existing normalized-coordinate send path and grant checks. Preserve a clear focus ring and announce placement instructions.

**Verification:** With a stream fixture and allowed grant, use keyboard only to place a point and a highlight; assert the sent annotation coordinates and the visible marks. Verify Escape sends nothing, revoking permission cancels pending placement, and Tab exits the surface. Exercise at phone and desktop sizes. These checks are proposed, not executed.

**Dedup/ownership:** `RoomAnnotations.svelte` has no changes in Claude's inspected design diff. D3-01 concerns Product mockup notes, a separate component and interaction; it does not repair this room surface.

## D5-02 — Minor — Kubeconfig checkboxes are exposed as a listbox without options

**Location:** `ui/src/modules/kubernetes/ClusterWizard.svelte:221`.

**Scenario and evidence:** Open Add cluster → From kubeconfig with discovered contexts. The container declares `role="listbox"` and `aria-multiselectable="true"`, but its children are labels containing native checkboxes; none has `role="option"` or an option selection state. The implementation supplies ordinary checkbox tab/Space behavior, not listbox option navigation.

**Impact:** The accessibility tree advertises a composite selection widget whose required option structure is absent. Assistive-technology users receive misleading widget semantics despite the underlying checkboxes being operable. This is a structural markup finding; a specific VoiceOver announcement was not measured.

**Smallest fix:** Keep the native labelled checkboxes and change the wrapper to a labelled `role="group"` (or fieldset/legend), removing `aria-multiselectable`. A custom listbox implementation is unnecessary for this flow.

**Verification:** Seed at least two contexts, open the wizard, inspect the accessibility tree, and run the accessibility helper. Verify no listbox required-child violation and that Tab/Space selects each checkbox with its context label and checked state exposed.

**Dedup/ownership:** Claude's inspected ClusterWizard diff changes semantic colours and logical padding, not the context-list roles. No matching application-review finding was present.

## D5-03 — Minor — Phone token rows hide the only textual expiration status

**Location:** `ui/src/modules/settings/PersonalAccessTokens.svelte:457`; status markup at `:234`, alternate visual treatment at `:399`.

**Scenario and evidence:** Open Settings → Personal access tokens at a viewport at or below 640 px with an expired token. The phone rule sets `.col-exp` to `display: none`, hiding both the expiry date and the sole “Expired” text. The remaining label/prefix cell contains no expiry information. Expired rows only differ by `opacity: 0.6`; the Revoke control remains enabled and identically labelled.

**Impact:** The compact layout drops the reason the token no longer works. Screen-reader users lose expiration information entirely, and sighted users must infer an undocumented state from dimming. The source establishes information removal; no contrast ratio is asserted.

**Smallest fix:** Include a compact textual expiration status in the token metadata on phone layouts, especially an explicit “Expired” label. Preserve the desktop date column and avoid announcing duplicate statuses at desktop widths.

**Verification:** Seed one active and one expired token. At 390 px verify both rows expose readable, distinct expiration information without horizontal overflow; repeat with screen-reader/accessibility-tree inspection and at desktop width to catch duplicate text. Check the compact badge in light and dark themes.

**Dedup/ownership:** No PersonalAccessTokens change was present in Claude's inspected design diff and no matching application-review finding was present.

## Coverage and limits

- **Settings/plugins/auth:** Sampled Login, token creation/list layout, Skills Library focus treatment, and PluginFrame loading/missing/error states. PluginFrame already has inline retry and a labelled frame; not reported again.
- **Usage/Insights:** Sampled model chart/table and provider controls, ccusage error handling, usage-report load states, and ReportDetail mode controls/HTML loading/error states. No chart contrast or rendered-layout claims are made.
- **Home/Share:** Sampled Home spaces, global arrow-key handler, widget state patterns, InsightsBox, and Share error/OTP branches. Home has an existing arrow-key path; it was not reported as keyboard-inoperable. Claude's existing Home/Share changes remain separately owned.
- **AWS/Kubernetes:** Sampled AWS region selection plus the separate design diff, namespace picker clamp/keyboard path, and ClusterWizard context controls. Athena region ownership remains C5-05. External cloud operations were not executed.
- **Rooms:** Read participants, chat, shared-screen layout, annotation tools and their parent mounting path. No media capture, two-device, microphone, screen-sharing, or real room operations were performed.
- **Assistant/personal agents:** Sampled PermissionsTab and AgentActivity, plus Claude's in-progress Assistant/personal-agent file coverage. Existing task cancellation/approval/takeover and cache findings remain with C5/P5.
- **History/proof/shared:** Sampled HistoryPage focus/responsive rules, ProofPage list/detail selection and load states, and shared Modal focus/scroll handling. No new finding asserted from these samples.

This was a finite source review of representative controls and states, not an exhaustive review of every settings panel, AWS service, chart, proof action, room flow, or theme. Runtime light/dark/RTL/phone screenshots and actual assistive-technology checks remain verification work for the visual owner.
