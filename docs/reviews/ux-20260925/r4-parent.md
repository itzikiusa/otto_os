# Round 4 — parent cross-checks

## API initial-load draft ownership: suspected issue rejected

Round3's `openApiEditor` helper waits for saved-request discovery because an untouched editor can become the empty-workspace onboarding view. That timing fix must not conceal draft loss. A new independent browser regression holds the real workspace's requests-list GET, types an actual URL into the visible editor, releases the empty response, and verifies that the URL remains and onboarding does not replace it. **Both Chromium and iPhone WebKit passed**. The existing `isDirty` condition protects the accepted draft; no production patch was justified. This differs from the separately reproduced/repaired first-run coach lifecycle defect.

From `ui/`: `OTTO_E2E_SLOT=ux4parent OTTO_E2E_PORT=7871 OTTO_E2E_PW_PORT=5371 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod npx playwright test e2e/desktop-ux-r4-parent-api.spec.ts --project=desktop-browser --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-r4-parent-api-results`. Log `/tmp/otto-ux-r4-parent-api.log`:2passed25.8s, exit0. No outgoing API request is sent; fixture URL is `fixture.invalid`. All daemon writes are the isolated workspace fixture.

## Product queued saves across login changes: reproduced and repaired

The content reviewer reproduced rejected autosave and lost multi-artifact drafts. An independent parent check held one artifact PUT, queued a second, switched the login token, then released the first response. The old implementation sent both requests: the queued edit inherited the next identity's bearer. The red run recorded `Expected: 1; Received: 2` in `/tmp/otto-ux-r4-parent-product-red.log`.

The Product repair captures a content generation, invalidates pending drafts on authentication changes, and checks the generation before queued transport and response application. A second parent regression confirms logout cancels an unsent debounce. Combined with the earlier Canvas identity regressions, **8 checks passed in Chromium and WebKit**, exit0, 45.1s: `/tmp/otto-ux-r4-parent-product-verified.log`, results `/tmp/otto-ux-r4-parent-product-verified-results`.

Command from `ui/`: `OTTO_E2E_SLOT=ux4parent OTTO_E2E_PORT=7860 OTTO_E2E_PW_PORT=5360 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod npx playwright test e2e/desktop-ux-r4-parent-product.spec.ts e2e/desktop-ux-r3-canvas-identity.spec.ts --project=desktop-browser --project=iphone-portrait --workers=2 --output=/tmp/otto-ux-r4-parent-product-verified-results`.

An initial combined run passed7/8: the mounted Canvas editor test incorrectly expected an edit action on the intentionally read-only phone preview. Inspection of the rendered screenshot and `CanvasPage`'s explicit readonly condition established the supported boundary. That edit/debounce case now uses834px in WebKit; the queue tests remain at their original phone viewport. The preview's overlapping Ask AI and zoom controls are a separate visible finding assigned to the content reviewer, not dismissed as a test error.

Parent review also caught an integration hazard in the first draft-retention patch: returning local drafts from every `attachmentText` call would hide a server live update whose content needed fetching. Draft restoration is now explicit at editor load; live-update reads still fetch server content. The content reviewer is adding its behavioral regression and checking conflict-dialog identity ownership.

## D2 WebKit initialization and production policy

The content reviewer reproduced D2 initialization failing in WebKit while the same diagram worked in Chromium. A readiness guard exposed the underlying Go/WASM stack overflow. Both the reviewer and parent independently reconstructed the WASM data segments, Go function metadata and element table: the repeating function was `regexp/syntax.(*compiler).compile`, with `rune` at the top. This was not a diagram parse failure. An isolated test of the canonical newer package did not repair it, so the dependency was not upgraded.

The parent ran the exact installed runtime on the main thread as a control: WebKit initialized and produced13,372 bytes of SVG in363ms; Chromium in527ms (`/tmp/otto-d2-main-thread.mjs`, `/tmp/otto-d2-main-thread.log`). The implemented fallback keeps those runtime globals in a private iframe and uses a private MessageChannel, retaining the normal worker on engines where it initializes. The existing serialized render queue is preserved. Parent review additionally required terminal transport failures to invalidate the cached renderer before queued work retries, and persisted pagehide to preserve the suspended renderer. The reviewer separately verified bounded timeout/retry and persisted-pagehide behavior in both engines.

The parent then built the production UI and served it with the **unchanged desktop content-security policy** read from `apps/desktop/src-tauri/tauri.conf.json`. API and WebSocket traffic went through a loopback proxy to a throwaway daemon; no live daemon or user data was modified. All five theme variants of real D2 rendering, source editing, syntax-error recovery, saved-source verification and Design Hall preview/editing passed in Chromium and WebKit: **10/10,51.0s,exit0**. Final log `/tmp/otto-ux-r4-d2-production-verified.log` contains no captured production console errors; `.last-run.json` is passed with no failures. The build passed in24.88s (`/tmp/otto-ux-r4-d2-production-verified-build.log`).

The first production run was7/10 because a parent fixture ran `localStorage.setItem('otto_base', location.origin)` in every frame. The about:blank renderer shares storage but has `location.origin === 'null'`; the fixture changed API requests to `/null/api/v1/...`, yielding404. Network traces proved the cause. Restricting that setup to the top frame made the complete unchanged-policy run pass; no product workaround was added for the faulty test.

Reproduction artifacts: `/tmp/otto-ux-production-csp-server.mjs` (static production server5371, proxy to isolated7871), `/tmp/otto-ux-r4-production-d2-verified.spec.ts` (temporary copy of the content fixtures with top-frame API-base setup), and `/tmp/otto-ux-r4-d2-production-verified-results`. Command used the normal harness with `OTTO_E2E_SLOT=ux4prod`, port7871, Playwright port5371, matching worktree debug daemon, `--project=desktop-browser --project=ipad-portrait --workers=1 --grep 'D2 real renderer'`. The temporary duplicate spec was removed from the repository and the parent's server stopped after verification. Later unrelated UI repairs still require the final integration build; this is specific production D2/policy evidence.


## Final-round scope and handoff verification

On26 September the user explicitly made round4 the last round, canceled round5 and reconfirmed PR → all Actions green → admin merge with `fix/git-tabs-use-free-width` → rebuild/reinstall/replace. There will be40 fresh scope reviews in total. Scores remain evidence-based; remaining limitations are not hidden.

The parent inspected the unfinished shell/content handoff logs rather than accepting their draft reports. Three WebKit Retry fixtures had bypassed `page.route` through the service worker; blocking the worker in that route-fixture suite preserved the original failure/recovery assertions. Four Files source assertions compared layout-dependent `innerText` newline serialization; they now compare actual CodeMirror line text before/after attempted input, while retaining `aria-readonly=true`. Phone Zoom in was a real defect: rendered15px stayed15px after the first click. Applying the stored zoom offset above the15px phone baseline fixes it. The combined14 cases (including the unchanged Swarm breakpoint regression) passed both engines,1.9m, exit0: `/tmp/otto-ux-r4-parent-handoffs.log`.

A further Notes identity regression was reproduced: a queued draft from the old login was sent with the replacement bearer after the earlier write completed. The first fixture accidentally masked it with an unauthorized workspace-list response; a second setup used a nonexistent per-workspace GET. The corrected fixture allows both identities to read the same workspace through the actual list contract and failed with two PATCHes instead of one (`/tmp/otto-ux-r4-parent-notes-red3.log`). Notes now capture the token when typed, verify it before queued dispatch, and ignore obsolete completions. The workspace read/merge/write helper also checks the captured token around its awaited read and response. No production user data was used.

Final parent selection: Notes identity, ordered save, failure/retry, retained focus; initial coach draft; tablet Canvas width/toggle and real D2 rendering passed14/14 across Chromium and WebKit,1.3m, exit0 (`/tmp/otto-ux-r4-parent-final.log`). The Canvas width had independently failed374px versus required>580; its bounded tablet scene list now leaves614px for the editor. Additional visual inspection found the Mermaid source pane still narrow and exposed an invalid new icon name; these follow-up repairs are not yet included in that passing result.

Full UI check passed zero errors/warnings and all454 unit tests passed at the earlier handoff revision (`/tmp/otto-ux-r4-handoff-check.log`, `/tmp/otto-ux-r4-handoff-unit.log`). Final revision gates remain required after all scope edits settle.

## Final content integration

After the earlier14-case parent selection, the final Mermaid source-width and fitted-SVG assertions reproduced the257px source pane. Container stacking and automatic fit repaired it. All17 final R4 content checks passed in WebKit and Chromium (`/tmp/otto-ux-r4-parent-content-final.log`, `/tmp/otto-ux-r4-parent-content-chromium.log`). The final Warm-light RTL tablet capture was visually inspected.

Product Arena's five main flows passed WebKit (`/tmp/otto-ux-r4-parent-arena.log`,1.2m,exit0). The initial combined run was21/22: page.reload followed immediately by a second goto aborted startup requests. The trace and Playwright's WebKit adapter proved these console CORS cancellations were reported as pageerror. The fixture now opens Arena in the reloaded document, retaining actual reload/persisted-byte/error assertions. The prior untraced R3 Load failed is not relabeled as this event.

The final Git settled2048px screenshot was inspected: five synthetic long repository tabs fill the header, plus remains visible and branch rail is loaded. The PR Summary/Files retained containers have no CSS display override of their hidden attribute. Cloud Native-dark Requests and Warm-dark RTL overview were independently viewed: readable data, focus ring and correct numeric/path direction. One extra trailing blank line in the new radio helper was removed after cached diff checking detected it; no behavior changed.

## Design checklist review

Reviewed the shared UI checklist against the whole branch and rendered evidence. New module CSS has no literal hex colours or px text sizes. The standalone HTML mockup template's11px label is self-contained and meets the minimum. The ARIA window splitter retains a justified Svelte separator/tabindex suppression; Snip's canvas application role has a local explanation and executed keyboard placement/select/move/delete checks. App drag-region suppressions moved with the stable shell and remain for native titlebar dragging. These exceptions must be called out in the PR. No new ui-guards allow comments were introduced.

Selected synthetic screenshots were individually inspected and copied into `evidence/`; no host transcript/account metadata is published. The final integration check exposed one new schema tablist focus warning, assigned back to the data reviewer instead of accepting a warning-bearing gate.454 unit tests and6 mocked deployment tests passed. Full Rust tests/clippy/rebuild are in progress; these results are not yet claimed.

## Kafka integration follow-up

The parent caught an introduced pointer regression in the new keyboard inspection control: ordinary table-cell clicks no longer selected a message. The data reviewer reproduced it and restored delegated primary-pointer selection while preserving the native keyboard button, text selection and right-click behavior. The schema tablist now has tabindex=-1, keeping one selected-tab stop and clearing the Svelte warning.

Further source tracing found Mask applied to the initial Peek but omitted from incremental Live reads. The service masks only when the request flag is true; a clock-driven browser test reproduced the missing flag. Incremental reads now carry the selected mask flag and a mixed buffer cannot falsely claim all its payloads are masked. Six final Chromium/WebKit tests passed1.0m (`/tmp/otto-ux-r4-data-final-followup.log`). The final data report includes this closure.

The combined UI check then passed zero errors/warnings (`/tmp/otto-ux-r4-final-check.log`,exit0). Production build passed32.28s and454 unit tests passed (`/tmp/otto-ux-r4-final-build.log`, `/tmp/otto-ux-r4-final-unit-verified.log`,exit0). Build chunk-size/ineffective-dynamic-import advisories remain, with no build failure. The style baseline was ratcheted only downward across eight file/rule counts; no debt limit was raised.

The first full Rust compile exposed a missing io-error conversion in the new collector materialization call; it was corrected to the existing domain Internal error convention before the fresh run. The repository-wide formatting check remains advisory and reports54 files of drift; no broad reformat is part of this UX branch. Final Rust/CI outcomes follow after completion.

The final full phone inventory passed165/165 checks in6.0m,exit0 (`/tmp/otto-ux-r4-final-inventory.log`). It runs all33 current routes for content height, horizontal overflow, critical accessibility, light mode and RTL against the isolated daemon. Command: from `ui/`, slot ux4parent, ports7860/5360, matching debug binary, `npx playwright test e2e/pages.spec.ts e2e/theme.spec.ts e2e/rtl.spec.ts --project=iphone-portrait --workers=2 --timeout=90000 --output=/tmp/otto-ux-r4-final-inventory`. The90s test limit accommodates cold compilation; actual per-page checks were generally2–6s. This complements loaded per-scope regressions; it is not evidence of every main workflow.

## Final reviewer closure

All forty fresh scope reviews are complete across four rounds. Help passed68/68 Chromium/WebKit checks; Insights passed36/36 final desktop checks and6/6 selected WebKit checks, plus deterministic collector calendar tests. The parent independently inspected the final light/dark desktop and RTL tablet report screenshots. The remaining calibrated dimensions below9.5 are preserved in the summary; final-round status does not erase design tradeoffs or untested native behavior.

Insights generation now pins an acceptance-date collector without rewriting installed custom skills. Manual generation explicitly regenerates an existing report; scheduled catch-up remains idempotent. The full workspace test compilation began before the last manual/scheduled prompt distinction, so a focused final-source Rust rerun is required after the ongoing workspace pipeline.

## PR handoff gate

The complete `cargo test --workspace` run passed3,565 tests with zero failures and66 ignored, including doc tests (125 test-result groups), `/tmp/otto-ux-r4-final-rust-verified.log`. It includes the pinned-collector regression but predates compilation of the last manual/scheduled prompt distinction. Clippy, the matching debug build and a final-source focused Insights run follow; their completion and GitHub Actions are release gates tracked on the PR. No merge or installation is claimed by this source checkpoint.
