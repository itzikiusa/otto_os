## Lens: Foundations — re-score

**Score: 8.5/10.** Start 10.0, minus 2 major (0.6), 7 minor (0.7) and 5 nit (0.15) = 8.55, which I round down to 8.5. Partial items count at their original severity.

I checked each item at HEAD b751ca6f by reading and grepping the code, with no builds. Where a "Fixed" rests on a count that dropped to zero, I say so.

| # | Previous finding | Status | Evidence |
|---|---|---|---|
| 1 | [major] Uppercase labels with ~10 letter-spacings | **Partial** | `letter-spacing` is now consistent across the shell: no off-scale values remain in `ui/src/shell/*`, so Navigator and Palette are done. About 25 off-scale values remain in 21 files. Examples: `SharePage.svelte:494` (0.18em, an OTP field, so legitimate), `VaultPage.svelte:596` and `OpenApiView.svelte:342` (0.4px), `GraphView.svelte:4337,4425,5038`, `ConflictHunk.svelte:392`, `SqsView.svelte:586`, `RdsView.svelte:356`. |
| 2 | [major] ~38 focus rules on raw `--accent` | **Fixed** | Zero `:focus*` rules with `border-color: var(--accent);` remain (was 38). `GridView.svelte:1203-1205` now uses `--accent-text` plus the 3 px halo. |
| 3 | [major] Off-grid spacing, including shared primitives | **Partial** | Modal, ContextMenu, FloatingBar, FolderPicker, ResourceAccess, Toasts, PageHeader and DoneContractMeter have no off-grid `padding` left. Off-grid `gap` fell from 573 to 548 across 265 files. There are still no `--sp-*` tokens in `tokens.css`, and no off-grid-spacing ratchet. |
| 4 | [minor] Warm light `--accent-text` contrast | **Fixed** | `tokens.css:249-251` pulls the Warm light mix to 50% toward `--text`, with a comment pointing at `tokenContrast.test.ts`. I did not recompute the ratio. |
| 5 | [minor] Weights 650 and 700 | **Fixed** | No `650` or `700` weights remain. `DocsAgentsView.svelte:1239` is now `600`. |
| 6 | [minor] Off-scale font sizes | **Partial** | Still present: `Chart.svelte:260,274` (34 px and 28 px), `TermKeysBar.svelte:144,172,176`, `BrokersPage.svelte:940,971,992,1014`, `SchemaTree.svelte:1048,1058`, `DatabasePage.svelte:2750,2810`, `AppliedPreview.svelte:166,221`, and em sizes (0.88–0.92em) in `LearningsView`, `DiscoveryTab`, `DiscoveryChat`, `RefineChat` and `McpServers`. The 16 px mobile-input cases are intentional. |
| 7 | [minor] Radius literals and chat-bubble shapes | **Partial** | The 18 px and 14 px bubble radii and the `BottomNav` 14 px sheet are gone, so the bubble inconsistency is fixed. Still present: 8 px and 5 px literals equal to tokens (`app.css:80`, `Navigator.svelte:1650`, `NotificationBell.svelte:510`, `DocsAgentsView.svelte` ×8, `ResultsGrid.svelte:2404,2419,2431`), `ReviewPanel.svelte:2175` (20 px), and 4 px and 6 px in `WorkflowsPage.svelte:294-295` and `panels/FileTree.svelte:241-242`. |
| 8 | [minor] Literal colours and missing on-scrim token | **Partial** | `--on-scrim: #ffffff` is defined at `tokens.css:325`. `Composer.svelte:644-645` and `Lightbox.svelte:35,59` now use `--scrim-media` and `--on-scrim`. Baseline `color-literal` is down from 24 to 20 files, but literals remain in `AnalysisTab`, `StickyNode`, `Inspector` (scene3d) and others. |
| 9 | [minor] Dead fallbacks and physical shorthand | **Partial** | The `Terminal.svelte:3270-3272` fallbacks are gone and `PageHeader` shorthand is fixed. One `token-fallback` (`DeviceFrame`, a user-content preview) and one `physical-shorthand` (`Terminal.svelte`) remain in the baseline. |
| 10 | [minor] Motion tokens barely adopted | **Fixed** | `--dur-fast` / `--dur-enter` usage went from 7 to 196 occurrences. A `transition-literal` ratchet exists with an empty baseline. |
| 11 | [minor] Private `@keyframes` in shared components | **Fixed** | `shell/` and `lib/components` have no `@keyframes` left. 21 module files stay baselined. |
| 12 | [minor] Hand-rolled shadows and legacy `--shadow` | **Partial** | Only three literal shadows remain: `ProductPage.svelte:1254`, `ResultsGrid.svelte:2354` and `DeviceFrame.svelte:85` (the last is a device preview). Legacy `var(--shadow)` use grew from 49 to 55 occurrences, so the "retire `--shadow`" part is Open. |
| 13 | [minor] Content headings above the title scale | **Partial** | `PrDetail` no longer uses `--fs-xl` for the title. `Walkthroughs.svelte:520` and `ReaderView.svelte:346` still use `--fs-2xl`, and the docs carry no exception. |
| 14 | [minor] Form-control boundary contrast | **Fixed** | `--control-border` is defined (`tokens.css:110,201,247,376`) and `app.css` references it in 3 places. |
| 15 | [nit] Status tokens duplicate tone tokens | **Partial** | Light now aliases `--status-working/exited/warn` to the tone tokens (`tokens.css:170-173`). Dark still has two ambers (`--status-warn` `#e0a000` vs `--warning` `#e3b341`), and `--status-idle` (`#69696e`) is still near `--text-dim` (`#636368`). |
| 16 | [nit] Negative letter-spacing on titles | **Open** | It spread from 6 to 14 non-`site.css` occurrences in 14 files (`PageHeader`, `Navigator`, `Login`, `Settings`, `Onboarding`, `HomeToday`, `Lobby`, …). It is consistent now but still undocumented. |
| 17 | [nit] Vault graph hard-coded fallbacks | **Open** | `GraphView.svelte:213` and `:1026` are unchanged. |
| 18 | [nit] NUL byte in `DiagramView.svelte` | **Open** | Grep still reports a binary match near offset 5693, and `.badge { font-size: 8.5px }` is unchanged at `:798`. |
| 19 | [nit] `--sp-*` and `--agent` tokens still proposed | **Open** | There is no `--sp` or `--agent` in `tokens.css`. |

### New findings

None of substance. The changes introduced no regressions I could find. Two growth items are tracked under #12 and #16 above and not re-counted:
- Legacy `--shadow` use rose from 49 to 55.
- Negative letter-spacing rose from 6 to 14 files.

### Remaining to reach 9.8

- Add `--sp-*` tokens and an off-grid-spacing ratchet. 548 off-grid `gap` declarations remain, and the shared primitives are already clean.
- Finish the typographic cleanup:
  - Replace the ~25 stray `letter-spacing` values with `.06em`.
  - Move the remaining off-scale px/em sizes onto `--fs-*` (`Chart`, `TermKeysBar`, `BrokersPage`, `SchemaTree`).
  - Resolve the two `--fs-2xl` content headings (`Walkthroughs.svelte:520`, `ReaderView.svelte:346`).
- Swap the 8 px and 5 px radius literals for `--radius-m` / `--radius-s`, and fix the `ReviewPanel.svelte:2175` 20 px. Move `WorkflowsPage` and `FileTree` preview radii onto tokens or document them as exceptions.
- Retire legacy `var(--shadow)`: alias it to `--glass-shadow` / `--shadow-card` and migrate the remaining uses. Replace the three literal shadows.
- Finish the small leftovers:
  - Strip the `DiagramView.svelte` NUL byte.
  - Tokenise the dark `--status-warn` / `--warning` amber split.
  - Remove the vault graph fallbacks.
  - Document or standardise title tracking.
