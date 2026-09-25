# Round 3 — integration cross-checks

These checks supplement the ten fresh reviewers; they do not count as another review agent.

## Canvas identity isolation

The R3 save queue could submit an older queued draft using a replacement login token. A browser regression held the first save, queued a second, changed the token using the actual client auth event, then released the first request. Before repair it observed two outgoing writes instead of one (`/tmp/otto-ux-r3-canvas-identity-red.log`). This was a regression in the new queue, not a speculative finding.

The Canvas store now invalidates its draft/save context when the login token changes. Editors capture that context on mount, so delayed timers and unmount flushes cannot restage or submit the previous identity’s content. Pending save/delete operations check context after waiting; old responses cannot add errors or overwrite the new editor. Drafts, old scene state, and pending navigation are cleared. Already-dispatched requests retain their original credentials and may finish; they cannot update the replacement UI. Scene-list and open responses are guarded against the same transition.

Final tests exercise both a queued save/delete sequence across a new identity and a real mounted Mermaid editor with pending text at logout. They assert outgoing request counts, removed old content, and absence of old error state. Existing failed-save recovery, same-identity queued edits across scenes, and Excalidraw hand-edit recovery still pass. Tests use isolated daemon state and intercepted writes; no real account is changed.

- Initial combined identity and existing Canvas checks: **10 passed**, Chromium + WebKit (`/tmp/otto-ux-r3-canvas-identity-verified.log`).
- Final identity checks plus workflow save regressions: **10 passed**, Chromium + WebKit (`/tmp/otto-ux-r3-parent-final.log`). Four cases are identity checks, six are existing instruction/graph-save checks. This also covers queued deletion cancellation.
- Main sources: `ui/src/lib/stores/canvas.svelte.ts` and its three file-backed editors. Regression: `ui/e2e/desktop-ux-r3-canvas-identity.spec.ts`.

## Workflow response compatibility

The combined UI check found four errors in the new version guard: `Workflow.version` is optional for older responses. Comparing absent versions as zero preserves monotonic updates without assuming the field exists. The six final workflow save/navigation checks above pass. Subsequent type checking reports no Workflow or Canvas errors; unrelated in-progress Git/Data findings are assigned to their owners. No whole-round UI gate is claimed yet.

## Rust workspace

The first full run found a stale macOS-only browser-install test that expected installation to fail despite the repository now providing verified checksum pins. The test now seeds a temporary installed fixture and verifies the idempotent installed response; unsupported platforms still reject. It neither downloads nor launches a browser. The focused test passed before the full rerun.

`CARGO_BUILD_JOBS=3 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace` passed: **3,562 passed, zero failed, 66 ignored**, across 125 test binaries/doc-test suites (`/tmp/otto-ux-rust-tests-verified.log`, exit 0). Full clippy with the same profile and `--workspace --all-targets -- -D warnings` passed (`/tmp/otto-ux-clippy-verified.log`, exit 0).

These Rust results precede the later R3 Git reference-normalization repair. That repair requires its own focused checks and renewed final gates; these results must not be represented as verifying a later Rust revision.

## Provider failure copy

The Git reviewer’s failed-reply screenshot exposed a misleading shared banner promising that retries would resume automatically. `serviceHealth.svelte.ts` tracks response status and a dismissal timer; it does not retry requests. The banner now states the observed failed provider request and that local work remains available. The reviewer re-executed the failed-reply flow in the final 13-test Git subset and visually verified the corrected text (`/tmp/otto-ux-r3-git-final.log`). No request/retry behavior changed.


## Later integrated gates and production worker check

- Full Rust workspace rerun after Git ref normalization: **3,563 passed, zero failed, 66 ignored across125 suites**, command `CARGO_BUILD_JOBS=3 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 cargo test --workspace`, log `/tmp/otto-ux-r3-rust-workspace.log`, exit0. This precedes the later Insights report-key extension; its focused `cargo test -p otto-server insights::tests` separately passed12 tests including the calendar-boundary regression (`/tmp/otto-ux-r3-insights-rust.log`).
- Production UI build passed (`/tmp/otto-ux-r3-production-build.log`). A temporary Playwright diagnostic used the actual hashed production bundle, enabled service workers, isolated daemon7860, and a local static/proxy server5360. It verified controller activation, populated asset cache, no cached API/WS entries, offline shell reload, visible Retry, and successful reconnection. **Both Chromium and iPhone WebKit passed** (`/tmp/otto-ux-r3-production-worker-outage.log`; `.last-run.json` passed). Server outage used socket destruction only in the disposable loopback test server; the user's daemon/network was never stopped. The temporary repository spec was removed after completion; diagnostic sources remain `/tmp/otto-ux-production-worker-verified.spec.ts` and `/tmp/otto-ux-production-server.mjs`.
- First diagnostic used Playwright `context.setOffline(true)`: Chromium passed, WebKit raised an internal navigation error. A minimal worker returning constant HTML reproduced the same WebKit error independently of Otto (`/tmp/otto-ux-webkit-offline-control.mjs`, `.log`). Therefore the replacement used an actual isolated HTTP outage in both engines; no product patch or hidden skip was made. This checks fresh install/reload/offline recovery, not a transition between two different production asset versions.

- Integrated UI units initially failed5 router-guard cases (449 passed): the R3 reactive share-token map added a dependency after its initialization and the source harness lacked that import. ESM browser imports are hoisted, but the test harness's CommonJS transpilation is sequential. Moved imports before initialization and supplied the real `SvelteMap` dependency to the fixture; no assertions weakened. Full rerun **454 passed**, `/tmp/otto-ux-r3-unit-verified.log`. Latest pre-final-wave full check also passed0errors/0warnings (`/tmp/otto-ux-r3-final-wave-check.log`).
