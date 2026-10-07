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
