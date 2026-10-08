# Quality review — round 1 integration checkpoint

Historical round 1 snapshot. The [final campaign report](FINAL.md) supersedes this checkpoint for current assessments, final integration results and delivery status. Original measurements and limitations below remain unchanged as provenance.

Base: `7cff9e538deccdd0e346af7c6285e11863285c00` (refetched `origin/main` remained unchanged). Branch: `review/quality-20261008-next`. Nine reviewers completed the targeted stages of round 1, one PR. The earlier five-iteration claim was incorrect; see the corrected [round ledger](PLAN.md). No production deployment or merge occurred during review.

## Result and score boundaries

All confirmed findings in the final changed-code review were repaired. **The requested whole-product score of 9.8 in every vertical was not established at the round 1 checkpoint.** Scores are scoped engineering judgments, not percentages derived from passing tests. Missing evidence is not represented as a discovered defect.

| Vertical | Round 1 independent changed-code assessment | Specialist assessment |
|---|---:|---|
| Performance | 9.2/10 | Browser repaired scope 9.2; terminal runtime met its measured envelope |
| Correctness / bugs | 9.4/10 | Evaluator 9.4 after independent repairs |
| Design | 9.3/10 | Populated browser/shared UI 9.5 |
| UX | 9.2/10 | Populated browser/shared UI 9.4; native composition 9.3 |

The conservative independent scores and narrower specialist scores are both retained. The higher UI scores cover observed DB/API/shared-shell journeys; they do not certify the whole product. Final gates establish integration confidence, not a numerical promotion to 9.8.

## Repairs

- Browser: bounded retained CDP output/events, pending calls, guard workers and proxy sockets; cancellation releases owned resources. An independently reproduced 128-request cached burst regression was repaired without raising admission limits.
- Evaluator: malformed/incomplete pass sets cannot produce passing review evidence; validator origin survives completion ordering; initial/retry publication preserves concurrent human ratings, updates the current winner, and counts failed validators in visible retry badges.
- Goal Loops: reject invalid mixed settings before any write; persist valid settings with one atomic SQL update, including storage-error rollback.
- Shared UI: keep focused actions reachable across overflow, recover workspace-load failures with Retry while suppressing stale identity errors, and load Proof deep links only after the workspace scope is ready.
- Native verification: extend the isolated School probe and replace its weak detached-frame assertion with settled visible/unfocused sampling. Production native behavior was not changed.

## Exact-source gates

| Check | Final result |
|---|---|
| Workspace rustfmt | Passed |
| Full workspace Clippy, all targets, `-D warnings` | Passed, Rust 1.99.0 |
| Full workspace nextest, CI profile | **5,469 passed, 92 skipped**, 278.441 seconds |
| Nextest override-filter guard | Passed with Python 3.13 |
| Workspace documentation tests | Command passed; no runnable doctests, one ignored example |
| Workspace build | Passed; fresh debug daemon SHA-256 `ef5b20835f96449cb06f55ec757099b206fe63576df3cad6b37a90ec0441c187` |
| Native standalone gates | Formatting, all-target Clippy and **55 tests passed** |
| Fresh-daemon browser/native checks | **15 browser tests passed**; stronger native SPA/School probe passed |

Full test fixtures used isolated Git config (`GIT_CONFIG_GLOBAL=/dev/null`, `GIT_CONFIG_NOSYSTEM=1`), without altering user config. The first filter-guard invocation used system Python without `tomllib`; its corrected Python 3.13 invocation passed. macOS debug linking emitted an existing compact-unwind section-size warning; the build and tests succeeded. Ignored/opt-in external-service, native trust and scale coverage is not represented as executed by the default workspace test count. Hosted mandatory checks remain separate.

Console logs are retained verbatim, including their original trailing spaces and terminal formatting; the source/document whitespace check excludes the evidence directory. Exact changed-source and final daemon hashes: [manifest](evidence/final/source-manifest.json). Final logs: [Clippy](evidence/final/clippy.log), [nextest](evidence/final/nextest.log), [filter guard](evidence/final/filters-py313.log), [doc-tests](evidence/final/doctests.log), [build](evidence/final/build.log).

UI source gates: Node 26.10.0; 1,823 unit tests passed, Svelte/TypeScript/guards clean, production build and bundle budget passed. A fresh type check after the populated E2E additions passed. No thresholds, security defaults, or ignore lists were relaxed.

## Measured and visual evidence

The independent 28-case populated/browser run passed without retries or skips. It uses the current UI but the baseline daemon; [the report](design-ux-iteration4.md) states which DB responses were mocked and which Git operations used real disposable repositories. Earlier overlapping 12-case selections are not added as unique coverage. Eight light/dark desktop/phone screenshots are linked there.

The uncontended terminal workload rendered 856/856 markers at one, three and five real PTY sessions, with pooled p95 input-to-render latency 27/35/35 ms and all queues drained. Its baseline daemon and actual Node 22 worker differ from the final-source/Node 26 integration gates. RSS/CPU variability, collector contribution and mounted recovery limits are in [performance iteration 3](performance-iteration3.md). This is a low-bandwidth six-minute workload, not a browser flood or leak-proof soak.

Native lifecycle, zoom and monitor-fit probes used actual isolated WKWebViews and temporary data. Strict current native AX acceptance was unavailable/failed under the process's existing permissions; no setting or trust prompt was changed. An earlier modal Cancel failed to restore visibility and a later diagnostic-only run passed. The earlier failure is **unexplained, not fixed**. The old detached frame sample did not establish visible/unfocused rendering; that claim is corrected in the original report. **Final exact-source native probe: passed.** Against the rebuilt daemon and bundled production UI, it passed real native zoom/draft/focus retention and three modal hide/resume cycles with no hidden frame growth. The stronger detached assertion first places only its own fixture windows side by side, waits for `!document.hasFocus() && !document.hidden`, settles 200 ms, then requires more than one fresh frame across 600 ms while both samples remain visible/unfocused. It passed, retained the same scene/room on Return, and logged `hidden:false, focused:false, frames:63`. This closes the old detached-test evidence gap for this bounded run; it does not explain the earlier modal timeout or supply AX/VoiceOver coverage. [Native log](evidence/final/native-final.log), [55 tests](evidence/final/native-tests.log), [Clippy](evidence/final/native-clippy.log).

The final **15/15 browser checks** ran on Node 26.10.0 against the rebuilt daemon: both new design/populated specs and `desktop-page-chrome.spec.ts`, one worker, isolated ports 17830/5203, no orphan sweep, no retries/skips. [Execution log](evidence/final/browser.log) and [JSON results](evidence/final/browser-results.json). No production/UI changes followed these checks.

## Remaining limits

No current VoiceOver journey, signed-distribution/install acceptance, hours-long native/browser soak, real-provider workflow, populated live-screen School recovery, all-module populated visual sweep or actual Chromium flood against the repaired daemon. These boundaries prevent a credible whole-product 9.8 claim. The native modal observation remains a reliability uncertainty; no speculative production repair was made.

## Review records

- [Plan and iteration ledger](PLAN.md)
- [Browser performance repairs](performance-iteration1.md)
- [Evaluator first pass](correctness-iteration1.md)
- [Shared design and UX repairs](design-ux-iteration1.md)
- [Independent browser/UI review](independent-browser-ui-iteration2.md)
- [Native acceptance and failed observations](native-iteration2.md)
- [Evaluator independent review](evaluator-iteration3.md)
- [Terminal runtime](performance-iteration3.md)
- [Populated design/UX](design-ux-iteration4.md)
- [Final independent review](final-independent-iteration5.md)
- [Coordinator findings and red/green evidence](coordinator-findings.md)

## Delivery

At this checkpoint PR #97 returned to draft while full review/fix rounds 2–5 proceeded. This record remains the verified round 1 snapshot; later source changes have their own round records and final integration evidence.

One PR contains the complete source, regressions, contracts and evidence. Merge to main and local rebuild/reinstall/replacement remain conditional on explicit approval and passing mandatory checks. Advisory outcomes are reported separately and do not become new merge requirements. Deployment must use the clean merged commit and the existing signed build/installed-process receipt workflow.
