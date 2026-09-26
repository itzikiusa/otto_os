# Round 2 — Content modules (reviewer 4/10)

Scope: Product, Vault, Canvas, Design Hall, Browser. All edits confined to `/Users/itziklavon/otto-ux-audit-20260925`; no commits, native runtime mutations, or external publishing. Feature store ownership: Vault's module store and `ui/src/lib/stores/canvas.svelte.ts`.

## Independently verified prior repairs

The previous round's complete `desktop-ux-content.spec.ts` passed, including Product phone onboarding + keyboard import dialog, Canvas phone assistant bounds/light/dark/close-with-diagram-retained, Browser RTL address direction, and Product/Vault/Canvas list failure → Retry recovery. Existing real isolated Browser mark/send-to-shell/save-to-vault, Design draft/edit/version/compare/restore/links/reference/menu, and Vault disk edit/search/graph-related/read/render flows passed. This is fresh runtime evidence, not acceptance of the old report.

## Repaired issues

- **P2 Browser keyboard marking.** `ui/src/modules/browser/ReaderView.svelte:62`: marking previously exposed only article click handling. Native-light keyboard regression failed because no composer appeared after Tab/ArrowDown/Enter. Mark mode now exposes rendered blocks as a roving keyboard selection with Up/Down/Home/End/Enter/Space and restores their original semantics on exit. The reader now uses shared `.md-body` styling: loaded long code is horizontally scrollable inside the block and stays LTR inside an RTL UI.
- **P2 Design Hall tab keyboard access.** `Lobby.svelte:264`, `ArtifactView.svelte:819`, `CompareModal.svelte:124`, local `tabKeys.ts`: all three tab groups now have roving tab stops, arrow/Home/End activation, and mirrored arrow direction in RTL. End failed to move focus before the repair. Runtime test creates an actual version and switches Compare tabs.
- **P2 Vault note failure recovery / stale requests.** `ui/src/modules/vault/vault.svelte.ts:516` and `VaultPage.svelte:408`: persistent inline failed-target/error/Retry/Dismiss banner preserves the previous note. Note navigation now owns a sequence before awaiting save, checks workspace as well as vault, ignores failures from obsolete requests, and checks for newly typed drafts after a delayed read before replacing the editor. The test injects 503, retries successfully, then releases an obsolete failing read after selecting another note. A first fixture assertion expected a title absent from the fixture body; corrected to the real note breadcrumb, not counted as a product defect. Visual review caught the new banner inheriting the center pane's flex growth; fixed to fixed-content height and added a <100px desktop bounds assertion.
- **P2 Canvas no-workspace dead end.** `ui/src/modules/canvas/CanvasPage.svelte:174`: Choose workspace now opens the shared filtered/clamped workspace menu or the existing workspace-creation sheet. The trigger is focused explicitly for WebKit focus restoration. Phone regression opens creation and returns focus with Escape.
- **P1 Canvas stale scene selection.** `ui/src/lib/stores/canvas.svelte.ts:114`: reproducible slow Alpha → select Beta → release Alpha replaced Beta's diagram. Request generations now guard both successful and failed opens; closing invalidates in-flight opens. Delayed response regression failed before the patch and passed after it.
- **P1 Design Hall typing during save falsely became clean.** `ui/src/modules/design-hall/ArtifactView.svelte:317,382`: saving a snapshot and typing newer text while the request was held caused Save to remain disabled after completion, silently treating newer unsaved text as persisted. Both ordinary and conflict-resolution save paths now pass the submitted content into `applySaved`. The held PUT regression failed before and passed after this two-call-site repair.

## Verification

All browser commands used `OTTO_E2E_SLOT=ux2content OTTO_E2E_PORT=7844 OTTO_E2E_PW_PORT=5344 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod`, from `ui/`, `--workers=1`.

1. `npx playwright test desktop-ux-r2-content.spec.ts desktop-ux-content.spec.ts desktop-browser-reader.spec.ts desktop-design-hall.spec.ts desktop-vault-docs.spec.ts desktop-vault-performance.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-content-verified` — **34/34 passed**, log `/tmp/otto-ux-r2-content-verified.log`, output `.last-run.json` records passed.
2. `npx playwright test desktop-ux-r2-content.spec.ts desktop-product-edit.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-content-final` — **11/11 passed**, log `/tmp/otto-ux-r2-content-final.log`. Includes loaded Jira fixture title/description edit → confirmation → persistence → reload → cancel; no real Jira writes.
3. `npx tsc -p tsconfig.e2e.json --noEmit` — passed again after the final test additions; log `/tmp/otto-ux-r2-content-tsc.log`.
4. `node ui/scripts/ui-guards.mjs` — passed, 803 files, no new debt. `git diff --check` for owned files passed. Parent owns full npm check.
5. Earlier red-run evidence: `/tmp/otto-ux-r2-content-before.log`, `/tmp/otto-ux-r2-content.log` (Canvas stale-selection failure), `/tmp/otto-ux-r2-content-draft-before.log` (Design save snapshot failure). Early phone Reader fixture used a hidden Go button; changed to the real phone Enter-in-address navigation, not counted as a product finding. Separate workspaces per Reader variant avoid an unmocked existing-tab navigation PATCH and keep screenshot data synthetic.

## Actually viewed screenshots

- `/tmp/otto-ux-r2-content-browser-native-light.png`: loaded long Reader + saved keyboard mark.
- `/tmp/otto-ux-r2-content-browser-native-dark.png`: loaded 375px phone Reader + marks.
- `/tmp/otto-ux-r2-content-browser-warm-light.png`: loaded 1024px RTL Reader; code remains LTR.
- `/tmp/otto-ux-r2-content-browser-warm-dark.png`: loaded desktop Reader.
- `/tmp/otto-ux-r2-content-browser-pro-dark-dark.png`: loaded 375px RTL Reader.
- `/tmp/otto-ux-r2-content-vault-retry.png`: last good note preserved beneath compact failed-target Retry banner (viewed again after fixing flex growth).
- `/tmp/otto-ux-r2-content-design-compare.png`: real saved versions, keyboard-active Changes tab (viewed again after waiting for shared sheet animation).
- `/tmp/otto-ux-r2-content-canvas-no-workspace.png`: phone CTA with returned focus.

## Per-family scores (not inflated to target)

| Page / variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive | Reason for ceiling |
|---|---:|---:|---:|---:|---:|---|
| Browser Reader, native light/dark, Warm light/dark, Pro Dark; desktop + phone + tablet RTL | 9.5 | 9.5 | 9.5 | 9.3 | 9.5 | Keyboard and save-focus-return flows verified in Chromium and WebKit; native live-browser mode and manual VoiceOver not covered. |
| Vault loaded read/edit/search, native desktop | 9.4 | 9.4 | 9.0 | 9.5 | 9.2 | Disk autosave and stale-read recovery verified; all-theme loaded-phone editor combinations not yet covered. |
| Canvas diagram/assistant/no-workspace, native desktop + phone light/dark | 9.4 | 9.4 | 9.2 | 9.5 | 9.5 | Diagram selection and onboarding fixed; paid generation and editable Excalidraw failure matrix not exercised. |
| Design Hall lobby/artifact/compare, native desktop | 9.4 | 9.5 | 9.4 | 9.3 | 9.1 | Actual create/edit/version/restore verified; full loaded phone and theme matrix remains untested. |
| Product empty/error phone + loaded Jira editing desktop | 9.2 | 9.4 | 9.1 | 9.3 | 9.2 | Jira simulated; no real integration, full discovery/mockup/refine/main-flow matrix remains untested. |

No whole-app or 9.5 claim. Native labels describe themes in browser renderings, not native Tauri execution. No VoiceOver/manual screen-reader pass. Remaining coverage is untested scope, not a claim that those combinations are defect-free.


## Final cross-browser extension

- WebKit fixture correction: `page.route` did not intercept cross-origin daemon traffic in the initial iPad project run. Changed this new spec to `page.context().route` and disabled service workers, matching the existing Jira-edit fixture convention. Those initial DNS/missing-mock failures were test setup issues, not product defects.
- A genuine Reader focus defect then reproduced: after saving a keyboard mark, “Mark passage” was inactive while focus had fallen to the body. Reader now explicitly returns focus to the mark button after successful save/stop; the Cancel button focuses the selected passage. The added save-focus assertion failed in `/tmp/otto-ux-r2-content-webkit-focus.log` before the repair.
- Natural click → Compare → Escape and Choose workspace → creation sheet → Escape passed in WebKit. No unnecessary Design focus patch was applied.
- Vault's additional delayed-read/failed-save case passes in WebKit: type into the existing editor while another note is loading, fail its save, release the read; the typed draft and original note remain.
- Parent full check reported zero errors and one new `workspaceEmpty` reactivity warning; fixed the Canvas binding to `$state()`. Parent owns the final whole-tree check.
- Viewed preserved WebKit screenshots `/tmp/otto-ux-r2-content-browser-native-dark-webkit.png` and `/tmp/otto-ux-r2-content-browser-warm-light-webkit.png`, showing focus rings, readable loaded phone content, contained code scroll, and tablet RTL. Five WebKit Reader screenshots are preserved with the `-webkit.png` suffix.
- The iPad project uses WebKit, with explicit desktop/phone/tablet viewport sizes in this spec. It is not evidence of an actual native iPad/Tauri installation.

**Final run completed:** `npx playwright test desktop-ux-r2-content.spec.ts --project=desktop-browser --project=ipad-portrait --workers=1 --output=/tmp/otto-ux-r2-content-complete` with the same environment above — **22/22 passed (11 Chromium + 11 WebKit)**. Log `/tmp/otto-ux-r2-content-complete.log`; `/tmp/otto-ux-r2-content-complete/.last-run.json` records `status: passed`, no failed tests. This includes the final Reader focus repair, compact Vault banner, reactive Canvas chooser binding, scene response ordering, and both draft-preservation regressions. Final E2E TypeScript and owned-file whitespace gates passed; UI guards passed with no new debt. No test process remains pending.

Final owned files: `ui/e2e/desktop-ux-r2-content.spec.ts`, `ui/src/modules/browser/ReaderView.svelte`, `ui/src/modules/canvas/CanvasPage.svelte`, `ui/src/lib/stores/canvas.svelte.ts`, `ui/src/modules/design-hall/{Lobby,ArtifactView,CompareModal}.svelte`, `ui/src/modules/design-hall/tabKeys.ts`, `ui/src/modules/vault/{VaultPage.svelte,vault.svelte.ts}`. Shared markdown rendering changes made by the accessibility reviewer are not authored or claimed here.

**Handoff:** no confirmed feasible defect from this review is intentionally left unfixed. Do not infer full coverage from that statement: use the per-family ceilings and explicitly untested workflows above to deepen round 3. No 9.5 whole-scope claim.
