# Round 2 reviewer 5/10 — shared design/accessibility

Worktree `/Users/itziklavon/otto-ux-audit-20260925` only. All five audit rounds remain mandatory. No commits, full npm check, production mutations, feature-module edits, or user-data deletion. Read protocol, prior R1 report, R2 data handoff, AGENTS.md, design/accessibility/review guidelines; used systematic debugging and verification/TDD instructions.

## Confirmed repairs

1. **P2 — browser LSP silently failed before initialization.** `ui/vite.config.ts:7`, `ui/package.json:72`, lockfile. Actual API GET → Pretty JSON ResponseViewer (`response.json`, read-only) with advertised JSON capability produced the events.EventEmitter externalization warning and zero LSP messages in Chromium and WebKit. Declared `events` 3.3.0 and exact `events`→`events/` Vite alias provide the real browser EventEmitter implementation to the nested open-rpc 1.x transport. No feature disabled or warning suppressed; CodeEditor itself needed no change. Regression intercepts the real WebSocket transport, receives initialize/initialized/didOpen, returns capabilities and a diagnostic, then asserts the JSON diagnostic is rendered and no externalization warning appears. Parent independently built production and repeated the real fixture against preview: **6/6 passed** (LSP, pointer nested sheets, three-layer stack × Chromium/WebKit); `/tmp/otto-ux-r2-build.log`, `/tmp/otto-ux-r2-production.log`, output `/tmp/otto-ux-r2-production`.

2. **P2 — Safari pointer-opened sheets returned focus to the previous field instead of their trigger.** `ui/src/lib/dialogFocus.ts:5`, `ui/src/lib/components/Modal.svelte:65`. Reproduced New Session → pointer-click Browse → Escape: picker closed, Browse remained unfocused in WebKit (keyboard-opened R1 had passed). Shared helper remembers the activating control through that click's render flush; Modal and custom dialogFocus use it for return. Normal pointer focus behavior is unchanged. Browser regression proves both engines, plus Navigator → Add Workspace → Browse → folder picker unwinds exactly one layer per Escape and restores every actual pointer trigger.

3. **P2 — long inline Markdown paths clipped outside the reading area.** `ui/src/app.css:427`. A synthetic Skills Lab document with a long inline path overflowed its article by **2,896px** on RTL phone; screenshot visibly lost the beginning. Shared prose now uses `overflow-wrap:break-word`, keeping fenced code horizontally scrollable and table min-content widths intact. Initial `anywhere` candidate was visually rejected because it squeezed table headers into letters; final break-word screenshots retain readable columns. Tests check internal article overflow, not only page overflow.

4. **P2 — scrollable Markdown code/tables were unreachable by keyboard.** `ui/src/lib/md.ts:20`, `:76`. Axe identified serious `scrollable-region-focusable` on real long code blocks. GFM renderer adds trusted tabindex=0 to pre/table only after sanitization; tiny renderer emits focusable code blocks too. The sanitizer's input attribute policy is unchanged. Tests assert focus and clean scoped axe in both engines, and actual ArrowRight scrolling in Chromium.

5. **P2 — long text-only confirmation bodies were unreachable by keyboard.** `ui/src/lib/components/Modal.svelte:22`, `:135`. After fixing Markdown, scoped axe independently exposed the same serious accessibility defect on `.sheet-body`. Added tabindex only while the body overflows, with ResizeObserver/MutationObserver cleanup. Ordinary short sheets gain no extra Tab stop; existing initial control/autofocus priority is preserved. Verified ArrowDown scroll in Chromium, focus and scoped axe in both engines, with all long choice actions inside viewport.

## Independent verification and test honesty

Durable new spec: `ui/e2e/desktop-ux-r2-access.spec.ts` (8 tests, two engines). Five theme variants cover Native light/dark, Warm light/dark, Pro Dark; phone RTL, tablet RTL, short viewport and reduced viewport equivalent to 120% layout space. **Native WKWebView zoom itself was not tested.** Reading current App.svelte showed that browser CSS zoom had intentionally been removed; an initial localStorage zoom fixture therefore did not actually zoom. Final tests instead explicitly reduce viewport dimensions and state that limitation.

R1 shared Markdown bidi/logical spacing, long choice footer, nested picker Tab trapping, and drawer→workspace focus checks independently passed in both engines. Existing shell focus/prompt/palette/Home tests passed **16/16** in the corrected final shell run. The first shell WebKit Needs-you test missed its API fixture because its service worker could bypass interception; added only `test.use({serviceWorkers:'block'})` to `desktop-ux-shell.spec.ts`, as required by the established protocol. No Home production behavior changed.

A standalone baseline established that Playwright WebKit does not implement native arrow-key scrolling here: focusable 100px overflow div containing 1800px of text, focused then ArrowDown, yielded Chromium scrollTop=40 and both desktop/iPhone WebKit scrollTop=0. Thus WebKit assertions verify focus plus axe; Chromium additionally verifies actual native keyboard scroll. No production keyboard workaround was added for this test-engine behavior.

The wrong initial fixture label “Choose workspace folder” was corrected to source-backed “Choose project directory.” Early shell command path mistakes failed before writes and were corrected. No assertion was weakened to mask a product defect.

Common Playwright environment, from worktree `ui/`:

```sh
OTTO_E2E_SLOT=ux2access OTTO_E2E_PORT=7850 OTTO_E2E_PW_PORT=5350 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- Initial new LSP/pointer spec, both engines: **3 failed, 1 passed** (2 LSP failures, WebKit pointer focus failure); `/tmp/otto-ux-r2-access-red.log`, output `/tmp/otto-ux-r2-access-red`.
- Same tests after dependency/focus repair: **4 passed**; `/tmp/otto-ux-r2-access-green.log`, output `/tmp/otto-ux-r2-access-green`.
- Deep Markdown/stack first run: five genuine inline overflow failures, one incorrect picker fixture label. `/tmp/otto-ux-r2-access-deep.log`.
- Deep tests after wrapping and fixture correction, before axe expansion: **6 passed**; `/tmp/otto-ux-r2-access-deep-green.log`.
- First full new+R1+shell run: **37 passed, 11 failed**, `/tmp/otto-ux-r2-access-final.log`. Ten failures exposed inaccessible scrolling Markdown/sheet bodies via the newly added axe checks; one was the shell worker-fixture problem above.
- New+shell corrected run: **27 passed, 5 failed**, `/tmp/otto-ux-r2-access-verified.log`. All production assertions including scoped axe passed in Chromium; remaining failures were the proven WebKit native-scroll engine limitation. Shell **16/16 passed**.
- Final new+R1: `npx playwright test e2e/desktop-ux-r2-access.spec.ts e2e/desktop-ux-r1-access.spec.ts --project=desktop-browser --project=iphone-portrait --workers=2 --output=/tmp/otto-ux-r2-access-complete`; result appended below.
- `npx tsc --noEmit -p tsconfig.e2e.json`: passed, `/tmp/otto-ux-r2-access-types-final.log` empty.
- `node scripts/ui-guards.mjs`: passed, 803 files, no ratchet regression; `/tmp/otto-ux-r2-access-guards-final.log`. No baseline increases.
- `git diff --check`: passed. Parent owns full UI/global gates and production build.

## Rendered evidence and review

Viewed with view_image: red phone clipped-inline-path screenshot under `/tmp/otto-ux-r2-access-deep`; corrected Warm light phone Markdown; Native light compact and Native dark short sheet; Warm dark and Pro Dark RTL sheet; iPhone API response with actual diagnostic, under `/tmp/otto-ux-r2-access-green` and `...-deep-green`. Also inspected final Native light Markdown under `...-final`, confirming the final break-word choice preserves readable table headers. Final screenshots are under `...-complete` (inspection appended below).

These are synthetic seeded documents/requests and local isolated state. They contain no production operation evidence. Screenshot foregrounds are the actual rendered shared surfaces, not an empty route alias.

## Scoped scores / 10

Scores reflect inspected components and verified browser behavior, not whole page-family scores inherited from another reviewer. No known confirmed defect remains in the repaired/tested paths.

| Shared surface / important variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
|---|---:|---:|---:|---:|---:|
| API JSON CodeEditor, Chromium and WebKit LSP init/diagnostics | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Markdown Native light compact / Native dark short | 9.5 | 9.5 | 9.6 | 9.5 | 9.5 |
| Markdown Warm light phone RTL | 9.5 | 9.5 | 9.6 | 9.5 | 9.5 |
| Markdown Warm dark / Pro Dark tablet RTL | 9.5 | 9.5 | 9.6 | 9.5 | 9.5 |
| Shared long text/long choice sheets, five themes | 9.5 | 9.6 | 9.6 | 9.5 | 9.5 |
| Pointer and keyboard nested sheets/custom Navigator stack | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 |
| Plain English palette keyboard ownership (existing focused flow) | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |

## Limits / next fresh round

No native Tauri, physical VoiceOver, real language-server process, or external service mutations. LSP completion/definition/reconnect and editor lifetime under delayed responses are not claimed by initialization/diagnostics evidence. Actual OS/browser zoom and virtual-keyboard occlusion remain unverified. Scoped axe includes contrast and all WCAG A/AA findings on the seeded Markdown and sheets; it is not a whole-app accessibility certification or every custom accent.

Three-layer custom drawer→sheet→sheet is verified. A higher custom overlay above an existing Modal is not a proven reachable defect in this pass; no speculative focus-manager rewrite. Other page modules and overall application scores remain with their assigned reviewers. Parent's independent production verification is attributed above rather than represented as this agent's run.

## Final result

**32/32 passed (1.8m)** in final new+R1 suite across Chromium desktop-browser and iPhone WebKit. `/tmp/otto-ux-r2-access-complete/.last-run.json` reports `passed`, `failedTests:[]`. No test processes pending. Parent final full UI gate additionally passed **0 errors, 0 warnings**; parent production checks remained **6/6 passed**.

Final rendered evidence actually viewed under `/tmp/otto-ux-r2-access-complete`: Warm light phone WebKit short-sheet (visible keyboard focus ring and all actions); Native light compact Chromium short-sheet; Pro Dark RTL compact/tablet Markdown; Native dark short Markdown; Warm dark Markdown including readable table and focused code scroller. Final screenshots supersede the early `anywhere` table-layout candidate.

Files for parent integration: `ui/package.json`, `ui/package-lock.json`, `ui/vite.config.ts`, `ui/src/app.css`, `ui/src/lib/dialogFocus.ts`, `ui/src/lib/components/Modal.svelte`, `ui/src/lib/md.ts`, new `ui/e2e/desktop-ux-r2-access.spec.ts`, and fixture-only `ui/e2e/desktop-ux-shell.spec.ts`. No commits made.
