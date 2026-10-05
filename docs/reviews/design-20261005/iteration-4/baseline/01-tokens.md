## Lens: Foundations (tokens, colour, type, spacing, radii, elevation)

**Score: 7.9/10.** Start 10.0, minus 3 major (0.9), 11 minor (1.1) and 5 nit (0.15) = 7.85. Spacing, uppercase letter-spacing and hand-rolled focus rings are still systemic across modules. The type scale and colour tokens are in good shape.

I read the three foundation docs in `/Users/itziklavon/claude_ade-design/docs/design/guidelines/`, `ui/src/lib/tokens.css`, `ui/scripts/ui-guards.mjs` and its baseline, then grepped `ui/src`. I did not run the guards or any build.

**Verified from earlier passes (credit):**
- Colour tokens are well-formed. Light and dark tone values are split in `tokens.css:152-174` and Warm dark has its own contrast pair at `:249-252`. I hand-checked `--text-dim` on `--surface-3` for Pro Dark (4.83) and Native light (about 4.8) against the doc table, and both match.
- The type scale is nearly clean. Only about 24 px literals remain outside `--fs-*`, and the only sub-11 px font left is baselined in `DiagramView.svelte`. The ratchet baseline shows no `heavy-weight`, `accent-text`, `accent-fill`, `focus-accent` or `status-as-text` debt.
- The 4 px `.btn` / `.icon-btn` radius token usage in the shared components is clean. `lib/components` has no `radius-literal` baseline entries.
- Floating layers use `--glass-*` and `--z-*` tokens. Only 4 z-index literals remain, in `SplitNode` and `TiledView`.

### Findings

1. `[major] Uppercase micro-labels use ~10 different letter-spacings`
   - Locations: 275 `text-transform: uppercase` declarations in 170 files. Spacing values seen:
     - 0.02em: `MethodTag.svelte:32`
     - 0.03em: `TopicDetail.svelte:1133`, `GroupsTab.svelte:606`
     - 0.04em: `Walkthroughs.svelte:507`, `Chart.svelte:271`
     - 0.05em: `ShortcutsOverlay.svelte:62`, `StoryLinkCard.svelte:102`
     - 0.07em: `Navigator.svelte:1668,1873` (the sidebar's own label)
     - 0.08em: `DocsAgentsView.svelte:1504`
     - 0.1em: `GraphView.svelte:4991`
     - 0.4px: `VaultPage.svelte:584`, `OpenApiView.svelte:342,377`
   - Rule: foundations §2.1 allows one form only, `.section-title` (`.06em`, `app.css:456-463`).
   - Fix: replace the hand-rolled labels with `.section-title` or a shared `--ls-label` token. Navigator should use `.06em`.

2. `[major] ~38 hand-rolled focus rules use raw var(--accent) and skip the .input halo`
   - Locations (`border-color: var(--accent)` inside `:focus` / `:focus-within`):
     - `WorkflowsPage.svelte:3462,4057,4073,4359`
     - `DatabasePage.svelte:1846`, `GridView.svelte:1205`, `MongoFilterBar.svelte:141`
     - `QueryBuilder.svelte:1350,1554`, `ExportDialog.svelte:332`
     - `GraphSearchBar.svelte:232`, `FileList.svelte:260`
     - `Palette.svelte:788`, `BroadcastModal.svelte:173`, `FindInPage.svelte:446`
     - `AssistantComposer.svelte:179`, `ShareModal.svelte:504`, `HistoryPage.svelte:720`
     - Product, canvas, design-hall, swarm and skills-lab files (+~19; find them with `grep -rn --include=*.svelte -B3 "border-color: var(--accent);" ui/src | grep ":focus"`)
   - What is wrong: foundations §1.2 says input borders and focus use `--accent-text`. The canonical `.input:focus` (`app.css:240-244`) adds a 3 px 22% halo. Some modules copy it (`Switcher.svelte:114`, `GitPage.svelte:757`, `SftpBrowser.svelte:503`), but these ~38 show a thin border change only, so focus visibility differs between inputs. The `focus-accent` guard only matches `outline`, so it misses this.
   - Fix: reuse `.input` / `.input-group`, or add a shared `.focus-ring` rule. Extend the guard to `border-color: var(--accent)` inside `:focus*`.

3. `[major] Off-grid spacing, including in the shared primitives`
   - Locations: 573 `gap` declarations of 5, 7, 9, 10, 11, 13, 14, 15 or 18 px across 276 files. Shared primitives:
     - `Modal.svelte:207` (`14px 16px 10px`)
     - `ContextMenu.svelte:447,463` (`7px 10px`, `5px 8px 4px`)
     - `FloatingBar.svelte:1071,1156,1264,1283,1347` (`8px 7px`, `8px 10px`, …)
     - `FolderPicker.svelte:416,443,457,465`
     - `ResourceAccess.svelte:540,565`, `DoneContractMeter.svelte:86`
     - `Toasts.svelte:74`, `PageHeader.svelte:613`
   - Rule: foundations §3 sets the 4 px grid (2, 4, 6, 8, 12, 16, 20, 24, 32). Chrome is where the drift is most visible.
   - Fix: fix the shared primitives first (Modal header to `12px 16px`, menu rows to `6px 8px`, …). Then add a ratchet rule `off-grid-spacing`, and the proposed `--sp-*` tokens.

4. `[minor] Warm light accent-text probably misses 4.5:1 on tinted selected rows`
   - Location: `tokens.css:96` (`--accent-text`), `:234` (Warm accent `#0f9d58`).
   - Hand-computed, so please confirm with the unit test:
     - On `--surface-3` (`#e7e3dc`) about 4.3:1.
     - On `--accent-soft` over `--surface-2` (a selected row in a well) about 4.0:1.
   - What is wrong: the §1.4 table measures `--accent-text` only on `--bg`.
   - Fix: add `--accent-text` on `--surface-2` / `--surface-3` / `--accent-soft` to `unit/ambient.test.ts` or a token test. If it fails, raise the Warm light mix from 62% toward `--text`.

5. `[minor] Weights outside 400/500/600 that the guard misses`
   - Locations: `Markdown.svelte:208` and `TurnItem.svelte:370` (`font-weight: 650`), `DocsAgentsView.svelte:1297` (`font: 700 var(--fs-xs) …`).
   - Rule: foundations §2.1 says no 700+ in chrome.
   - Fix: use 600. Extend `heavy-weight` to match 650 and the `font:` shorthand.

6. `[minor] Remaining off-scale font sizes (px and em)`
   - Locations:
     - `Chart.svelte:260,274` (34 px and 28 px KPI numbers; `--fs-hero` is wordmark-only and `--fs-2xl` is 22 px)
     - `TermKeysBar.svelte:144,172,176` (14/18/16 px)
     - `BrokersPage.svelte:952,983,1004,1026` and `DatabasePage.svelte:2734,2794` (14 px)
     - `SchemaTree.svelte:1048,1058` (14 px)
     - `AppliedPreview.svelte:166,221` (17 and 14 px)
     - `LearningsView.svelte:917`, `RefineChat.svelte:300`, `DiscoveryChat.svelte:409` (0.88em), `DiscoveryTab.svelte:492` (0.9em), `McpServers.svelte:497` (0.92em)
   - The 16 px values on mobile inputs (`WipPanel.svelte:1442`, `ReviewPanel.svelte:2301`, `PrDetail.svelte:1015`) are intentional iOS no-zoom and fine.
   - Fix: map to `--fs-*`. Add a guard for non-token px sizes ≥ 11, with an allowlist for the 16 px mobile inputs.

7. `[minor] Radius literals and two different chat-bubble shapes`
   - Locations (46 baselined across 25 files):
     - Literals equal to a token: `app.css:80` (5 px), `Navigator.svelte:1643`, `NotificationBell.svelte:510`, `DocsAgentsView.svelte` (8 hits), `ResultsGrid.svelte:2404,2419,2431`, `OpenApiView.svelte:291,321,429`
     - Off-scale: `ReviewPanel.svelte:2175` (20 px), `BottomNav.svelte:267` (14 px)
     - Chat bubbles with different shapes:
       - `TurnItem.svelte:311,317` and `ConversationView.svelte:1207` use 18 px with a 5 px tail
       - `ChatView.svelte:363,370` (assistant) uses 14 px with a 4 px tail
   - Fix: swap the equal-valued literals for `--radius-s` / `--radius-m`. Bubbles become `--radius-l` with a `--radius-s` tail, defined once. BottomNav sheet becomes `var(--radius-l) var(--radius-l) 0 0`.

8. `[minor] Literal colours and a missing on-scrim token`
   - Locations: 58 hits in 24 files. Examples:
     - `Composer.svelte:645` (`#fff`)
     - `Lightbox.svelte:35` (`#000 78%`, while `--scrim-media` exists at 60%), `Lightbox.svelte:59` (`#fff`)
     - `AnalysisTab.svelte:1098` (`#1b1b1b`)
     - `StickyNode.svelte:79`, `PresentMode.svelte:410`
     - `InsightsBox` / `ReportDetail.svelte:684` (`white`)
   - Fix: add `--on-scrim: #fff` next to `--scrim-media` and use both in Composer and Lightbox. Keep the user-content previews as documented exceptions.

9. `[minor] Dead fallbacks and physical shorthand left in shared components`
   - Locations:
     - `Terminal.svelte:3270-3272` (`var(--border, #444)`, `var(--surface, #28282e)`, `var(--text, #e8e8e0)`; dark hex fallbacks on defined tokens)
     - `DeviceFrame.svelte` (1 hit)
     - `PageHeader.svelte` and `Terminal.svelte` (3 `physical-shorthand` hits)
   - Fix: drop the fallbacks and use `padding-inline` / `padding-block`. Both are baselined debt in shared components.

10. `[minor] Motion tokens barely adopted`
    - Locations: `--dur-fast` / `--dur-enter` are used 7 times against 172 `transition` declarations. Literal durations:
      - `app.css:114-117` (`.btn` itself uses `130ms ease-out`, not `var(--dur-fast)`)
      - `AttributionDrilldown.svelte:337` (`0.2s`), `SwarmPage.svelte:957` (`0.3s`)
      - `MissionControl.svelte:766`, `Terminal.svelte:3277`, `TermKeysBar.svelte:148`, `WorkGraphView.svelte:150` (`0.1s`)
    - Fix: change `.btn` and the other shared primitives to the tokens first.

11. `[minor] Shared components still define private @keyframes`
    - Locations: `Modal.svelte` (2), `StatusDot.svelte` (1), `Drawer.svelte` (3), `Palette.svelte` (2), `NotificationBell.svelte` (1), plus ~20 module files in the baseline.
    - Rule: foundations §8 says use `otto-fade-in` / `otto-pop-in` / `otto-pulse`.
    - Fix: migrate the shell and shared primitives first, since they own the entrance motion.

12. `[minor] Hand-rolled shadows outside the elevation ladder`
    - Locations: `ProductPage.svelte:1212`, `MockupAnnotations.svelte:453,483` (a 6 px 20 px floating shadow), `SpatialView.svelte:211,241`, `ColorsSection.svelte:225`, `ResultsGrid.svelte:2354`. 49 uses of the legacy `var(--shadow)` remain in 35 files.
    - Rule: foundations §5 allows `--shadow-card` and `--glass-shadow` only.
    - Fix: map floating panels to `--glass-shadow` and cards to `--shadow-card`. Retire `--shadow` by aliasing it everywhere.

13. `[minor] Content headings above the page-title scale`
    - Locations: `Walkthroughs.svelte:512` and `ReaderView.svelte:346` (`--fs-2xl`), `PrDetail.svelte:777` (`--fs-xl`).
    - Rule: foundations §2.1 reserves `--fs-2xl` for dashboard hero figures and `--fs-l`/600 as the one title size.
    - Fix: use `--fs-l`/600, or document a "document title" step. Reader and Walkthroughs are content, so a documented exception is acceptable.

14. `[minor] Form-control boundary contrast is below 3:1`
    - Locations: `tokens.css:184` (`--border`, 10% black), `:100` (`--border-strong`).
    - What is wrong: Native light `--border` over white is about 1.3:1 and `--border-strong` about 1.8:1. Inputs are bounded only by this hairline plus a `--surface-2` fill, below WCAG 1.4.11 for control boundaries.
    - Fix: add a `--control-border` at about 3:1 for `.input`, checkbox and select outlines. Keep `--border` for card hairlines.

15. `[nit] Status tokens duplicate the tone tokens`
    - Locations: light `--status-warn/working/exited` equal `--warning/--success/--danger` (`tokens.css:158-161`). `--status-idle` `#69696e` is within 3 levels of `--text-dim` `#636368`. In dark, there are two ambers: `--status-warn` `#e0a000` vs `--warning` `#e3b341`.
    - Fix: alias them (`--status-warn: var(--warning)`).

16. `[nit] Negative letter-spacing on titles has no rule`
    - Locations: `app.css:448`, `Navigator.svelte:1654`, `PrDetail.svelte:780`, `Walkthroughs.svelte:514` (`-0.01em`), `App.svelte:161`, `Chart.svelte:262` (`-0.02em`).
    - Fix: document one title tracking value or remove it.

17. `[nit] Vault graph canvas carries its own dark palette`
    - Locations: `GraphView.svelte:199-214`, `:1026`.
    - Detail: hex fallbacks `v('--accent', '#0a84ff')` and `ghostFill` `#77777f` / `#9a9aa2`. This is runtime token reading, so it is mostly OK, but the fallbacks are dark-scheme only.
    - Fix: read `--text-dim` for the ghost fill.

18. `[nit] DiagramView.svelte contains a NUL byte`
    - Location: grep reports a `\0` byte near offset 5693 of `ui/src/modules/database/DiagramView.svelte`, so it is treated as binary. I did not confirm what character it is.
    - It sits beside the only sub-11 px font left (`.badge` at `:798`, 8.5 px, baselined). Per the rule that is allowed only for decorative counters.
    - Fix: strip the NUL, then tokenise or justify the badge.

19. `[nit] Foundations still list items as "Proposed"`
    - Locations: the `--sp-1…8` spacing tokens (§3) and `--agent` identity colour (§1.3).
    - Why it matters: both are needed for findings 3 and 8's follow-up.

### What would get this lens to 9.8

- Add the `--sp-*` tokens and an `off-grid-spacing` ratchet. First fix the shared primitives (Modal, ContextMenu, FloatingBar, FolderPicker, ResourceAccess, Toasts, PageHeader).
- Define one `.focus-ring` / `--ls-label` and migrate the ~38 raw-accent focus rules and the ~275 uppercase labels (or `.section-title` them). Extend `focus-accent` to `border-color` in `:focus*`.
- Close the guard gaps: weight 650, the `font:` shorthand, off-scale px/em sizes ≥ 11, and any literal `transition` duration. Pay down radius, colour and font debt, since the baseline is still 46 + 58 + 24 entries.
- Measure `--accent-text` on all surfaces and `--accent-soft` in the unit test, and fix Warm light if it fails. Add a 3:1 `--control-border`.
- Move the shell and shared components onto the motion tokens and shared keyframes (`.btn`, Modal, Drawer, Palette, StatusDot).
- Collapse the redundant status/tone tokens, add `--on-scrim`, unify the chat-bubble radii, and retire the legacy `--shadow` and the hand-rolled shadows.
