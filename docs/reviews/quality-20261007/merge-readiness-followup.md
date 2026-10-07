# PR 94 merge-readiness follow-up

**Test-review verdict: Approve the bounded repair diff.** Blocker 0, major 0, minor 0. This is a fixture-quality review, not a replacement for the full post-merge review or a claim that remote CI is green.

The failing reference is [CI run 37634194991](https://github.com/itzikiusa/otto_os/actions/runs/37634194991), PR head `2d1b92446003ed6c1dde3e700d2c0bc3060de879`, tested as merge `aba0c71` into main `196048df5bbd55a691b093ea0a3fa456f0912674`. Local evidence below uses that head plus the uncommitted repair diff and a freshly rebuilt `target/debug/ottod` on 2026-10-07.

## Proven session and mobile causes

- `desktop-agents-retention.spec.ts` called `/app/kill-sessions` while the agent UI-control case was running on the same daemon. `SessionManager::shutdown_ids` retires every affected session's credentials (`crates/otto-sessions/src/manager.rs:5547`), so the next MCP catalog refresh correctly refused the dead credential. CI's two cases overlap at 14:38:04–14:38:10 UTC. Running those two specs with two workers reproduced the failure locally: the agent lost its advertised UI tools. Retention now kills only its own two session IDs and asserts both become exited. The same global kill was removed from `desktop-view-only-attach.spec.ts`; its dormant/view-only/input/classic-attach assertions remain.
- Global shutdown coverage remains in the isolated Rust tests `shutdown_waits_for_an_in_flight_status_write` and `shutdown_retires_every_session_within_its_budget_despite_held_locks` (`manager.rs:7251` and `manager.rs:7298`): real PTYs are retired, every status becomes exited, and no live handles remain. These tests were source-inspected during this follow-up, not rerun by this reviewer.
- `connections-mobile.spec.ts` assumed no MySQL chip could exist because its workspace seeded only SSH/custom profiles. DB profiles are global. An explicit MySQL profile created in a second workspace reproduced the old assertion failure, expected zero chips versus one. The repaired test verifies that global profile's chip, visibility under MySQL/All, exclusion under SSH/custom, and removal of a nonmatching populated section. It retains viewport and empty-folder checks.
- The original three `desktop-agent-ui-control.spec.ts` journeys and their permission, cancellation, result-targeting and headless assertions were not changed.

## Independent review of the remaining repair diffs

Read all eight changed E2E specs against the relevant production paths and fixture helpers. No new skips, retries, disabled assertions, or failure-budget increases were introduced.

- Redis ports and connection-ID selectors isolate workers and avoid selecting another fixture's globally named connection; the real-engine row, scrolling and accordion checks remain.
- The MySQL cancellation fixture holds a uniquely named table lock instead of invoking policy-blocked `SLEEP`. It observes the actual waiting server query before Escape, requires the native `cancelled` response and disappearance of that query while the lock remains held, and retains the loading/UI recovery assertions. A client-only abort or rejected SELECT cannot satisfy these checks.
- S3 follows the current unsupported-type preview and asserts downloaded filename plus exact binary bytes. SQS checks the confirmation's queue/body and the real count transition from three to four, then purge to zero. No AWS engine path was mocked.
- Automation waits for the destination page's own heading before inspecting shared empty-state text, preventing the outgoing page from satisfying the next route's checks.
- School regression cases retain keyboard actions, resulting view/selection changes, persistence and resource-disposal checks. Lower device scale reduces screenshot capture cost; the original full-resolution journeys remain configured separately. The software-renderer branch itself still fixes WebGL pixel ratio to one (`ui/src/modules/home/school/scene.ts:189`). These functional checks do not establish a full-resolution rendering performance result.

## Executed verification

| Check | Result | Local log |
| --- | --- | --- |
| Session/mobile: four named specs, iPhone portrait + desktop browser, two concurrent workers, fresh daemon | 10 passed, zero skipped, 41.6 s | `/tmp/otto-pr94-connections-fresh.log` |
| Same session/mobile batch before the rebuild | 10 passed, zero skipped | `/tmp/otto-pr94-connections-green.log` |
| Mobile deterministic reproduction | Old chip assertion failed after cross-workspace DB seed | `/tmp/otto-pr94-mobile-red.log` |
| Concurrent session reproduction | Original global kill removed agent UI tools | `/tmp/otto-pr94-mcp-red.log` |
| Redis mobile + MySQL shortcuts | 9 passed | `/tmp/otto-db31-fixed.log` |
| LocalStack AWS | 7 passed | `/tmp/otto-pr94-aws-green.log` |
| School regression group | 8 passed | `/tmp/otto-pr94-school-regressions.log` |
| Automation | 17 passed | `/tmp/otto-pr94-automation.log` |
| E2E TypeScript; diff whitespace | Passed | Executed directly during this review |
| Workspace strict Clippy | Passed | `/tmp/otto-pr94-clippy.log` |
| UI type/style, units, production build, bundle budget | Zero errors/warnings; 1,698 units passed; build/budget passed | `/tmp/otto-pr94-local-gate.log` |

The reviewer personally executed the session/mobile batch and TypeScript/diff checks; the remaining completed run logs were inspected. Expected engine errors in cancellation/headless-refusal journeys are exercised outcomes, not hidden test failures.

`advisory-red-baseline.json` lowers `max_failed` from 56 to **0**. PR 94 closure requires zero functional failures; the old allowance must not turn residual failures into a passing ratchet. Other broad cases, the new remote run across all shards, and the full post-merge review remain pending. The original failed CI result is not superseded by these focused local passes.

## Second CI follow-up

[CI run 37645751954](https://github.com/itzikiusa/otto_os/actions/runs/37645751954), head `228d7ebc223967afda0f984dcf4b706adc4f04a0`, completed with seven functional failures. The zero-failure ratchet correctly blocked it. Rust, UI, smoke, live drivers, macOS builds, security checks and both performance jobs passed. This run does not establish merge readiness.

The second repair batch addresses the observed causes:

- ClickHouse's completion popup covered the phone editor's click target. The query helper focuses the editor and dismisses completion before replacing SQL. A deliberately opened popup reproduces the old failure; the repaired six-case live ClickHouse suite passes (`/tmp/otto-pr94-ch-popup-red.log`, `/tmp/otto-pr94-ch-popup-green.log`).
- Git's initial fetch finished before the test installed its observer; the next active fetch was correctly due after 60 seconds. Observation now starts before navigation and requires a successful response for the exact repository with `behind: 1`. All six cases pass (`/tmp/otto-pr94-git-fetch-green.log`).
- Home's trace contained two versions of the shared Svelte runtime after Vite discovered a lazy dependency and reoptimized mid-document. Scanning lazy page/widget entries up front prevents that invalidation. A fresh-cache probe reproduced the changing dependency hash before the fix and verified a stable hash and working widget afterward (`/tmp/otto-pr94-vite-lazy-before.log`, `/tmp/otto-pr94-vite-lazy-after.log`).
- Classroom list journeys now load the persisted List view directly. Separate real-canvas journeys retain view-switching coverage. Home and classroom-list specs pass together, ten cases (`/tmp/otto-pr94-home-fixed.log`).
- The session-header assertion read colors before their CSS transition painted. A bounded assertion waits for the same active/inactive distinction and uniform inactive colors. All six cases pass (`/tmp/otto-pr94-session-header-fixed.log`).
- The School keyboard regression's initial projected pointer landed on the floating command bar in CI. It now selects the intended child through the supported accessible companion before exercising Enter/Space on the same card actions. Both keyboard cases pass; independent pointer journeys remain (`/tmp/otto-school31-fixed.log`).
- D2's test shortened every production deadline to 100 ms, including healthy recovery. It now advances a controlled browser clock through the unchanged 30-second stalled deadline, then allows a deliberately slower healthy reply. It still requires timeout, queued recovery, valid parsing and retained frame identity. All 17 content cases pass (`/tmp/otto-pr94-content-green.log`).

These are focused local results. A fresh complete remote run on the committed repair head remains required before merging.

Independent review of the second batch found no defects in the bounded repair diff: production deadlines, real protocol transport, pointer journeys, query outcomes and active-title distinctions remain covered; no skips, whole-test retries or failure-budget increases were added. The final local UI gate passed with zero type/style warnings, 1,698 unit tests, production build and bundle budget (`/tmp/otto-pr94-round2-gate.log`). Stable toolchain refresh and full workspace strict Clippy passed (`/tmp/otto-pr94-round2-clippy.log`).

## Third CI follow-up

[CI run 37651444439](https://github.com/itzikiusa/otto_os/actions/runs/37651444439), head `c3649722ed9663fdd85d2cd40d9f68c27ee53994`, closed all seven preceding failures. Functional shards recorded 1,676 passed, two failed, zero flaky results and 25 existing skips. Linux Rust executed 5,261 passing tests; the smoke, performance, macOS, driver, UI and CodeQL checks passed. The zero-failure ratchet correctly blocked this run too.

Both remaining failures were stale test readiness signals, confirmed against CI traces and local delayed-load reproductions:

- Floating command bar: session navigation changed the hash while the previous page still held focus. The test therefore skipped reopening the bar; the destination terminal subsequently mounted, took focus and docked the bar, hiding its intact thread. The test now explicitly holds destination module loading, observes the hash transition, releases the module, waits for the exact new session's terminal focus, and reopens the bar with the real keyboard shortcut. Its question, answer and Open-result assertions remain. The old focus decision reproduces the CI failure (`/tmp/otto-pr94-floating-red-control.log`); the entire final spec passes seven cases (`/tmp/otto-pr94-floating-fixed.log`). The held module is released in `finally`, and its content is never mocked.
- CLI update shortcut: session prefetch now starts before the workspace list resolves and `currentId` is assigned. The old sessions-request observer fired too early, so the shortcut correctly displayed “Select a workspace first.” The repaired test waits for the rendered current-workspace chip, opens the real confirmation and observes the exact owned workspace's update request. A temporarily delayed workspace response reproduces the original failure and passes after repair; the final undelayed two-case spec also passes (`/tmp/otto-pr94-cli-shortcut-red.log`, `/tmp/otto-pr94-cli-shortcut-delayed-green.log`, `/tmp/otto-pr94-cli-shortcut-green.log`). The endpoint remains intercepted to prevent actual host CLI installation.

An independent source/test review approved both repairs with zero findings. No product code, timeouts, retry counts or failure budgets changed. A fresh committed-head CI run is still required.
