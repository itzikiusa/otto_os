# Round 3 — Content review (fresh reviewer 2/10)

Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. Scope Product, Vault, Canvas, Design Hall, Browser Reader, and the Snip editor at `#/snip/<id>`. Shared Canvas store ownership explicitly granted by parent. No commits, subagents, real provider operations, native clipboard mutations, or real external writes. The E2E daemon used synthetic workspaces/files and its test clipboard sink. Native interactive screen capture and macOS permission prompts were not exercised or simulated as verified.

## Confirmed repairs

1. **P1 Canvas source drafts disappeared after failed save + navigation.** `ui/src/lib/stores/canvas.svelte.ts:75`, `MermaidCanvas.svelte:59`, `D2Canvas.svelte:67`, `ExcalidrawCanvas.svelte:230`. Reproduction: type source, switch before debounce, reject PUT, reopen scene. The old server source replaced the user's draft. New per-scene snapshots survive scene switches and failed requests; writes for the same scene are serialized, and only the current snapshot can become clean. File-backed editor saves use the shared queue; Excalidraw stages actual serialized hand edits before debounce. A rectangle survives a rejected save and scene switch. Old unmounted Excalidraw completions cannot write source into a newer mounted editor. `CanvasPage.svelte:171` keeps a persistent, named failure banner with Open draft and Retry save, including when the failed scene is no longer selected. Tests cover retry recovery, pending-save switch, blocked first PUT followed by newer Alpha and Beta edits, durable server contents, and failed Excalidraw hand edits. Browser-visible loss failed before repair in `before2`; held save ordering, two-scene results and retained rectangle pass afterward.
2. **P2 Reader marking retained nested link tab stops and a composer from a different page.** `ui/src/modules/browser/ReaderView.svelte:65`. A nested link was reachable inside the roving block selection (observed missing `tabindex=-1`). Mark mode now temporarily removes nested interactive tab stops and restores them afterward. The composer belongs to the fetched page; changing source clears it. A late save only clears the exact submitted selection on the same page. Keyboard selection and existing focus-return flows are independently regressed. This does not claim a physical VoiceOver pass.
3. **P2 Snip text-entry escaped the viewport near image edges.** `ui/src/modules/snip/SnipEditor.svelte:410`. On 375px phone, its right edge was **522px**; RTL tablet also failed. The input is clamped and size-capped inside the editor body; annotation image coordinates remain unchanged. Editable text has an accessible name and a minimum 11px size. Five-theme screenshot flows verify bounds. Phone tool hit targets measured **24px**, now at least 36px, with balanced wrapping; Copy/Close also get 36px heights.
4. **P2 Snip transient load errors falsely said the image was deleted.** `SnipEditor.svelte:85,614`. An intercepted503 produced “This snip no longer exists.” Only 404 now uses the missing state; other failures show their cause and Retry. Retry503→404 is tested. Image URLs arriving after destruction are revoked, and mounted snip identity is captured for cleanup saves. A real pending drawing→different snip regression verifies the original image receives the final copy; the first pass also exposed a Svelte destroyed-derived warning, removed by the captured identity.
5. **P2 Product Discovery could show Alpha's report inside expanded Beta.** `ui/src/modules/product/DiscoveryTab.svelte:53,68`. Hold Alpha detail, open Beta, release Alpha: the report switched to “Report alpha” under Beta's expanded row. List/detail generations now check both operation and selected story before applying success, failure, or loading completion. Changing stories clears expansion.
6. **P2 Product Design Arena left a usable desktop preview under 200px wide.** `ui/src/modules/product/design/DesignArena.svelte:803,1118,1645`. At1280px viewport, story list + fixed 250px assets + 300px inspector squeezed the artifact stage to about 198px; the actual Dashboard template screenshot showed only its own navigation sidebar. A container-sized layout now uses the existing Assets/Canvas/Inspector navigation below 960px available width, giving the selected artifact about 746px. The tab group supports roving focus, arrows, Home/End, and RTL direction. Desktop/tablet loads open the selected artifact on Canvas after breakpoint remount. Tests verify useful stage width, keyboard navigation, resize1280→1800→1024, retained loaded artifact, and pin annotation creation. Before/after screenshots were actually viewed.
7. **P3 Dashboard starter labels below the text floor.** `ui/src/modules/product/design/templates/dashboard.html:24`. Actual template creation + iframe computed style measured10.5px on KPI labels. Raised to 11px; the browser assertion now passes. This is a bundled standalone artifact template, not app chrome.

## Independent prior-repair verification

R1 Product phone empty CTA, Canvas assistant light/dark fit/close, Browser RTL URL direction, and Product/Vault/Canvas list-error Retry are rerun separately from loaded Product fixtures. R2 tests recheck Reader keyboard marking/focus, Vault note-error recovery and stale failed reads/draft preservation, Canvas no-workspace action and late selection response, and Design Hall tabs and typing during a held save. Existing Browser reader, Design Hall version/compare/restore/link, Vault real file editing, Snip draw/undo/copy/delete, and Product simulated Jira editing regressions are rerun as a separate daemon invocation.

The initial combined prior run is **not** used as passing evidence: a second invocation accidentally reused this slot and its failed setup tore down the first daemon, producing ECONNREFUSED. Separately, Product's globally listed loaded story fixture invalidated R1's empty-page assumption. The fresh R1-only invocation resolves that fixture contamination without changing or weakening its assertions. Initial new-test setup also used a button role name for the Mermaid “Code” control when “Edit the Mermaid source” is its title; corrected to getByTitle before accepting Canvas loss as a product reproduction.

## Verification commands and outcomes

All Playwright commands from `ui/` use:

```
OTTO_E2E_SLOT=ux3content
OTTO_E2E_PORT=7851
OTTO_E2E_PW_PORT=5351
OTTO_E2E_SWEEP_ORPHANS=0
OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

Fresh prior verification is complete: **7/7 R1** and **33/33 prior/main-flow regressions**. The corrected cross-engine run is complete: **26/26 passed (13 WebKit + 13 Chromium)**, `/tmp/otto-ux-r3-content-verified2/.last-run.json` is `passed` with no failed tests. The additional Product legacy-suite check is complete: **7/7 passed**, for **73 scoped checks** across these four sequential completed invocations; all four `.last-run.json` files say `passed` with no failed tests. Earlier new-scope completed runs: `final` **11/11 passed**; `complete` **11 passed / 1 failed** (new resize assertion found the Assets reset, subsequently repaired); `final-edge` **2/2 passed** (resize/annotation + pending snip switch). Logs all `/tmp/otto-ux-r3-content-<suffix>.log`, artifacts corresponding paths without`.log`.

- `npx playwright test desktop-ux-content.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-content-r1verified`
- `npx playwright test desktop-ux-r2-content.spec.ts desktop-browser-reader.spec.ts desktop-design-hall.spec.ts desktop-vault-docs.spec.ts desktop-snip.spec.ts desktop-product-edit.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r3-content-priorverified`
- `npx playwright test desktop-ux-r3-content.spec.ts --project=desktop-browser --project=ipad-portrait --workers=1 --output=/tmp/otto-ux-r3-content-verified2`
- `npx playwright test product-design-arena.spec.ts product-mockups.spec.ts --grep 'arena:|attach HTML|manual Import' --project=ipad-portrait --workers=1 --output=/tmp/otto-ux-r3-content-productverified`: **7/7 passed**; log `/tmp/otto-ux-r3-content-productverified.log`.
- `npx tsc -p tsconfig.e2e.json --noEmit`: passed; `/tmp/otto-ux-r3-content-tsc.log`.
- `node ui/scripts/ui-guards.mjs`: passed after using a feature-specific Snip action class; `/tmp/otto-ux-r3-content-guards.log`. No baseline raised.
- Owned-file `git diff --check`: passed. Parent owns full npm/build gates. Parent's initial full check exposed four widened CanvasDoc discriminants; explicitly typed them and notified parent.

## Rendered evidence actually inspected

All paths below are synthetic UI evidence under`/tmp/`:

- `otto-ux-r3-content-{vault,design,snip}-{native-light,native-dark,warm-light,warm-dark,pro-dark-dark}.png`: all15 viewed. Native dark/Warm dark are375px phone; Warm light is1024px RTL tablet; Native light/Pro Dark are1440px desktop (Pro Dark RTL). New runs replace files with the final rendered state. Design phone additionally chooses iPhone frame and actually edits/saves a second version.
- `otto-ux-r3-content-product-dashboard.png`: before/after actual dashboard template; new final stage reveals KPI/chart/table content.
- `otto-ux-r3-content-discovery.png`: expanded Beta stays Beta after held Alpha resolves.
- `otto-ux-r3-content-excalidraw.png`: retained rectangle + persistent save-error banner and reachable recovery actions.
- Snip entry images with `-entry.png` suffix show the clamped control itself; native-dark phone and Warm-light RTL tablet entries were actually viewed.

Theme names refer to browser-rendered app themes. WebKit device project overrides use explicit desktop/phone/tablet sizes; not a native Tauri or physical iPad test.

## Scores and remaining depth

Scores describe inspected quality and the evidence limits below; none is raised to meet the 9.5 target.

| Page / inspected variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive | Scope of evidence / next depth |
|---|---:|---:|---:|---:|---:|---|
| Reader desktop Native light / Warm dark |9.5|9.5|9.4|9.4|9.5|Prior main flows and new nested-link/source-change marking; overlapping mark-save/new-selection timing needs more depth|
| Reader phone Native dark / Pro Dark and tablet Warm light RTL |9.5|9.5|9.4|9.4|9.5|Prior five-theme keyboard/focus regressions; native live mode is outside these scores|
| Vault loaded desktop Native light / Pro Dark RTL |9.5|9.5|9.3|9.5|9.5|Real file editing, search, links, OpenAPI/D2/JSON, persistent note retry|
| Vault loaded phone Native dark / Warm dark |9.3|9.4|9.3|9.5|9.3|Readable loaded note; tree uses substantial vertical space; larger tree/search/edit combinations remain|
| Vault tablet Warm light RTL |9.4|9.5|9.3|9.5|9.4|Loaded note and navigation bounds inspected|
| Canvas Mermaid native desktop + phone assistant |9.4|9.5|9.3|9.5|9.4|Failed pending saves, queue ordering, source recovery, prior selection and phone assistant checks|
| Canvas Excalidraw native desktop |9.4|9.5|9.3|9.5|9.4|Actual hand-drawn rectangle, rejected save, reopen/recovery banner; no phone/full-theme claim|
| Canvas D2 |—|—|—|—|—|Persistence uses the shared repaired queue; no new D2 rendered workflow score in this round|
| Design Hall desktop Native light / Pro Dark RTL |9.5|9.5|9.4|9.4|9.5|Create/edit/version/compare/restore/link regressions plus loaded source edit/save|
| Design Hall phone Native dark / Warm dark |9.3|9.5|9.4|9.4|9.3|iPhone frame + actual source edit and second saved version; preview/details split remains dense|
| Design Hall tablet Warm light RTL |9.4|9.5|9.4|9.4|9.4|Loaded source edit/save and bounds|
| Product Discovery + simulated Jira editing desktop |9.4|9.5|9.4|9.4|9.4|Late-report regression and prior title/description persistence; agent-run completion matrix remains|
| Product Design Arena compact desktop/tablet |9.4|9.5|9.4|9.4|9.4|Actual Dashboard template, useful stage width, keyboard pane navigation, resize, pin annotation; multi-artifact unsaved breakpoint matrix remains|
| Product phone empty / error |9.4|9.5|9.4|9.5|9.5|R1 keyboard import CTA and list Retry freshly passed; not a loaded Discovery/refine phone score|
| Snip desktop Native light / Pro Dark RTL + tablet Warm light RTL |9.4|9.5|9.3 (controls)|9.5|9.5|Drawing/undo/copy plus decoded output, bounds and load Retry; native capture excluded|
| Snip phone Native dark / Warm dark |9.4|9.5|9.3 (controls)|9.5|9.5|36px toolbar targets, clamped text control, clipboard-sink result; keyboard-only drawing/selection and screen-reader drawing are unscored|

Snip accessibility scores apply to inspected named toolbar/text controls, keyboard tool shortcuts/undo, phone target sizes, and announced loading/copy status. Keyboard-only creation/selection of new annotations and screen-reader interpretation of drawn content were not independently exercised and remain unscored R4 targets, not a confirmed defect or an imposed score ceiling.

Further Canvas review targets: multiple failed scene banners on phone; coalescing repeated Retry clicks while pending; explicit workspace switching during a held save; same-scene reopen while old completion arrives; draft/cache cleanup across logout→different user. Retained drafts are in memory, and the banner says so; no persistence across app restart is claimed. CanvasPage currently mounts Mermaid/D2/Excalidraw; legacy CanvasEditor/CanvasFlow store timers were not claimed as a verified active editor path. Other future targets: full Product multi-artifact unsaved edit across breakpoint remount, Reader same-page reload while mark save waits, and long/reduced-motion/custom-accent variants. No whole-app completion or blanket9.5 claim.

The first cross-engine combined run finished **24/26**: both failures were duplicate Product story titles across projects (strict locator violation), while all 13 WebKit cases passed. All Canvas scene and Product/Excalidraw title fixtures are now unique per call/workspace. Discovery waits for the held response to finish and two render frames before the final assertion. Snip's pending-copy case decodes the outgoing PNG and proves 300×200 dimensions plus painted, nontransparent pixels from the annotation, as well as the original snip id.

Existing `product-design-arena.spec.ts` and `product-mockups.spec.ts` full-workbench cases now request 1800px to exercise the full three-pane variant; the new R3 spec explicitly tests the compact 1280/1024px variant. Assertions are preserved. Only their five Arena and two manual upload/import cases are selected for this review; provider-driven AI creation/refinement tests are not invoked.


## Runtime-log follow-up and final handoff

The supplemental seven cases passed, but their log contained a Three.js PCFSoftShadowMap deprecation and one `[Unhandled rejection] TypeError: Load failed` during the code-view autosave/reload case. It is **not classified as harmless or pre-existing**. A follow-up used temporary pageerror/requestfailed/reload-boundary instrumentation, then added fetch-callsite and unhandledrejection tracing. `npx playwright test product-design-arena.spec.ts --grep 'code-view edit' --project=ipad-portrait --workers=1 --output=/tmp/otto-ux-r3-content-reloadtrace` passed **1/1**; the same command with `--repeat-each=3 --output=/tmp/otto-ux-r3-content-reloadtrace2` passed **3/3**. Both `.last-run.json` files say `passed`. No failed fetch/request or unhandled rejection reproduced. These four diagnostics did capture `SecurityError: The operation is insecure` with stacks exclusively in `web-inspector://bootstrap.js` while sandboxed previews mounted. The original Load failed has insufficient stack evidence to establish reload cancellation versus a user-path rejection; retain as an R4 tracing target, not a confirmed repaired defect. Temporary instrumentation was removed. No production source changed during this follow-up.

Final totals: **73 scoped regressions plus four diagnostic reload repetitions passed**. UI E2E TypeScript and guards passed; owned-file diff whitespace passed. No test process remains in flight. Full npm/build/integration gates remain parent-owned. Snip's introduced destroyed-derived warning is absent from the corrected final cross-engine run; this does not assert that every existing dependency/runtime log is clean.

Changed production files: shared Canvas store; CanvasPage/MermaidCanvas/D2Canvas/ExcalidrawCanvas; ReaderView; SnipEditor; Product DiscoveryTab, DesignArena, and dashboard template. Tests: new desktop-ux-r3-content plus full-workbench viewport clarification in existing product-design-arena and product-mockups. No Vault or Design Hall production code changed in this round. No commits created.
