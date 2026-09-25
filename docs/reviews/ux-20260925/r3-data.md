# Round 3 — data tools, reviewer 7

Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. Read AGENTS, design overview/layout/accessibility/review checklist, R1/R2 reports and R3 parent pointers. No commits, subagents, production data writes or external service mutations. SQL/API/Kafka/SFTP transports use synthetic contract fixtures; profiles are newly seeded in the disposable daemon. The network-profile fixture now imports actual app tokens/styles and applies the selected theme.

## Prior repairs independently checked

All 22 preexisting R1/R2/network-profile tests passed in the combined run `/tmp/otto-ux-r3-data-final.log`: SFTP click/Space and phone controls; API phone response tabs; replay validation and confirmed exact mocked selector/evidence; group selection/preview races; saved/history loading/error/retry; query edit staging/discard without writes; SSH failed-save draft retention; topic search/error/Retry; profile conflict/save/workspace isolation. All five desktop theme variants plus phone and tablet RTL loaded screenshots were freshly generated and actually viewed.

The first prior run stopped after 11 passes and one API screenshot failure. Trace proved `openApiEditor` saw the transient editor during `apiClient.loading`, returned, then the empty-workspace onboarding replaced it before the URL fill. Parent granted shared `ui/e2e/helpers.ts:58` ownership: wait for “Loading saved requests…” to finish before selecting the onboarding CTA/editor. No production behavior or assertions were weakened. The later all-theme run passed. The combined run's sole remaining failure was a test selector using an anchored regex against whitespace in workspace button text; switching to the exact accessible name fixed the fixture, and the actual stale-response test passed.

## Confirmed findings repaired

1. **P2 — a delayed Save renamed/linked the wrong query tab.** `ui/src/lib/stores/database.svelte.ts:3403`. Start saving `SELECT 41`, create/edit a second tab `SELECT 42`, release the first response: before repair, original tab remained “SELECT” and the new tab became “Original query”. Capture initiating tab before await; update only that still-open tab, and reject stale workspace/access context for saved-list updates. Red `/tmp/otto-ux-r3-data-red`, green in complete suite.

2. **P3 — RTL arrows moved against visual tab order.** `DatabasePage.svelte:721,738`, `QueryEditor.svelte:756`. Schema→ArrowLeft selected Connections instead of Saved; query/main strips had the same physical-arrow assumption. Arrow progression follows each strip's computed direction; Home/End remain logical endpoints. Regression covers all three strips and focus/selection.

3. **P3 — phone Schema looked selected but was absent from the tab sequence.** `DatabasePage.svelte:990`. Choose desktop Connections, resize to 375px, expand Schema & saved. Schema had active CSS with `aria-selected=false`, `tabindex=-1`, and no selected tab. It now exposes the same logical alias as the visible content. Red `/tmp/otto-ux-r3-data-green` (the first initial-phone-only attempt correctly passed; the actual transition reproduced it); fixed in complete suite.

4. **P2 — completed Kafka offset reset replaced another group's detail.** `GroupsTab.svelte:244`. Confirm alpha reset, hold POST, open beta, release POST: beta displayed alpha. Snapshot group/cluster/request/body before confirmation; verify context before sending; apply returned detail only if still current; success names the group actually reset. Test sends only a fixture POST and passes the typed-name confirmation UI.

5. **P2/P3 — group split followed viewport instead of available pane width; divider lacked keyboard operation and used LTR drag arithmetic.** `GroupsTab.svelte:78,278,515,789`. At a 1440px viewport, constrain the content to 620px: before repair list and detail still competed horizontally; expected stacked detail y>=875, actual y=124. Wrap in an inline-size container and stack below 760px available content; cap wide list at 45%; preserve scrolling. Divider is focusable with value semantics, Home/End/Enter/arrow/Shift-arrow support and RTL-aware dragging. A narrowly documented Svelte suppression is for the valid APG focusable separator, per parent review; it does not remove keyboard behavior. Final narrow RTL screenshot viewed.

6. **P2 — network profiles accepted invalid TCP ports.** `NetworkProfiles.svelte:45,87`. Number inputs' min/max were not enforced by the button-driven save flow; -1, 65536 and 1.5 enabled Save. Shared derived validation guards both button and handler; blank/zero/negative/fractional/oversized ports rejected, valid5432 accepted. Existing conflict/retry/save tests remain green.

7. **P3 — technical text inherited RTL.** Parent-approved `ui/src/lib/components/CodeEditor.svelte:628` now applies `dir=ltr` only to the editor host, preserving mirrored toolbar/chrome. SQL and pretty JSON response computed direction are tested. `NetworkProfiles.svelte:76` preserves host/environment identifiers; `SftpBrowser.svelte:297` preserves filenames, symlink targets, sizes, timestamps and POSIX permissions. Fresh tablet screenshot had shown `MB 1.0` and suffix-first truncation; final SFTP assertions cover the real rendered fields. LSP production verification remains parent/shared-owned.

8. **P2 — tablet structure detail had only ~315px after two sidebars.** `SchemaTree.svelte:68`, `DatabasePage.svelte:536`. At834px, selecting a schema search result left the 219px shell and300px schema sidebars open: name/type fragments were technically scrollable but crowded. Selecting a structure on tablets now collapses the schema sidebar, focuses the selected Structure tab, and gives detail >500px. Show schema restores focus to the selected object/search hit and preserves its search. Regression verifies query draft across834→1440→375 resize. Before/after screenshot inspected; final `/tmp/otto-ux-r3-data-screenshots/schema-tablet.png`.

## Verification and evidence

All commands from `WT/ui`, prefix:

```sh
OTTO_E2E_SLOT=ux3data OTTO_E2E_PORT=7856 OTTO_E2E_PW_PORT=5356 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- `npx playwright test e2e/desktop-ux-r3-data.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-data-red`: 5 meaningful failures,1 initial-phone pass. Log `/tmp/otto-ux-r3-data-red.log`.
- Combined `desktop-ux-r3-data.spec.ts desktop-ux-r2-data.spec.ts desktop-ux-data.spec.ts desktop-network-profiles.spec.ts`:35 passed,1 workspace-selector fixture failure; all22 old tests passed. `/tmp/otto-ux-r3-data-final.log`.
- New saved-workspace retry, history-connection retry and120-object schema search tests:3passed42.6s. `/tmp/otto-ux-r3-data-deep.log`; `.last-run.json` passed.
- Entire new17-test R3 suite: **17 passed1.1m**, `/tmp/otto-ux-r3-data-complete.log`; `.last-run.json` passed, failedTests=[].
- Post-tablet composition repair: `desktop-ux-r3-data.spec.ts desktop-ux-r2-data.spec.ts --grep 'Large schema|Loaded data workbenches tablet-rtl|RTL API JSON'`: **3 passed37s**, `/tmp/otto-ux-r3-data-tablet-final.log`; `.last-run.json` passed. Includes focus/return and draft-preserving resizes.
- Final seven loaded-theme/device screenshot tests (after SFTP direction repair):6passed; RTL assertion initially selected the translated header “Size”, not file metadata. Corrected it to rows containing the file/directory button; independent RTL rerun **1passed24.4s**, `/tmp/otto-ux-r3-data-sftp-final.log`, output `.last-run.json` passed. All seven variants therefore passed and screenshots were refreshed; final corrected RTL SFTP image was viewed.
- `npx tsc --noEmit -p tsconfig.e2e.json`: passed, `/tmp/otto-ux-r3-data-tsc-final.log` empty.
- `node scripts/ui-guards.mjs`: passed,803files,no regressions, `/tmp/otto-ux-r3-data-guards-final.log`. No baseline changes.
- `git diff --check`: passed. Full npm check/build remain parent-owned.

Screenshot paths: reused R2 spec freshly overwrites `/tmp/otto-ux-r2-data-screenshots/{variant}-{database,api,brokers,replay,sftp}.png`; these are R3 executions, not acceptance of old images. All database/API/broker/SFTP five-theme desktops viewed with view_image, plus phone and tablet examples; replay Pro Dark viewed and all seven replay variants generated by behavioral tests. New `/tmp/otto-ux-r3-data-screenshots/`: all seven network-profile variants viewed; groups-narrow-rtl viewed; schema-tablet before/after viewed. Network-profile screenshots are an isolated component fixture with real tokens, not a fabricated full app route.

## Scoped scores

Scores judge inspected browser UI and verified fixture behaviors. They are not claims that every protocol, engine, automation or real cloud operation was exercised.

| Page / important flow | Layout/readability | Interaction | Accessibility | States/recovery | Responsiveness | Overall |
|---|---:|---:|---:|---:|---:|---:|
| Database query/results/saved/history |9.5|9.6|9.5|9.6|9.5|9.5|
| Database schema search/Structure tablet |9.5|9.6|9.5|9.5|9.6|9.5|
| API request/response JSON main flow |9.5|9.5|9.5|9.5|9.5|9.5|
| Kafka groups/topics/replay |9.5|9.6|9.5|9.6|9.5|9.5|
| Connections SSH form/SFTP browsing |9.5|9.5|9.5|9.5|9.5|9.5|
| Network-profile manager component |9.4|9.5|9.4|9.6|9.4|9.4|

For the first five rows, Native light/dark, Warm light/dark and Pro Dark1440×900 all score9.5 for the reviewed flow. Phone375×812 Native light and tablet1024×768 Warm dark RTL score9.5 for those same loaded workbench flows; Structure's specific834×1112Native-light push-detail scores9.5 after repair. Network-profile component at all five desktop themes, phone375×812 and tablet834×1112Warm-darkRTL scores9.4: readable and operable, but the full-width stacked form with a separately scrolling60vh manager remains less composed than the actual workbench surfaces. This score is not raised to match the target. Full embedded network-profile launch/management contexts were not visually covered here.

## Remaining depth / limits

- No known confirmed bug remains in the repaired/rechecked paths. Do not extrapolate this to every data-tool surface.
- Next round should exercise saving while switching CONNECTIONS (this round covers changing query tabs and workspace-scoped list retries), repeated/double submission, and saved-query update/delete while selection changes. These are further test targets, not asserted findings.
- API automation execution, SSE/WebSocket/gRPC modes; DB builder/ERD/dashboard mutations and all engine-specific key/collection editors; Kafka produce/consume/schema registry; long SFTP transfers/cancellation remain unscored deeper paths.
- Continue NetworkProfiles embedded main-flow composition and multi-endpoint touch/keyboard review; current fixture validates actual component rendering and save/conflict behavior but does not cover all modal/session hosts.
- Native Tauri/VoiceOver, production cloud writes and production LSP bundling were not performed here; parent owns aggregate gates and final integration. Contract-faithful mocks establish the reviewed UI behavior without claiming real-service success.

Final handoff: all scoped tool processes exited. Final TypeScript and diff checks passed; final UI guards passed. Parent may copy this report into docs/reviews/ux-20260925/r3-data.md and integrate the verified changes. No aggregate app/production gate claim is made here.
