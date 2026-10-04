# Iteration 1 implementation — orchestration and MCP

Scope: C4-01 through C4-06, all five P4 findings, UX4-01 through UX4-03, D4-01. Changes are on the shared review branch; no commits, builds, test executions, provider launches or live-user-data writes were performed by this implementer. Parent owns central verification and runtime measurements. Systematic-debugging and test-driven-development guidance informed tracing and regression selection; tests were authored but no local red run was observed. Source traces, not failed fixture compilation, establish the original bugs.

## Changes and acceptance mapping

| Finding | Implementation | Regression / status |
|---|---|---|
| C4-01 approval caller shadowing | `McpApprovalRepo::find_usable` filters the nullable requester explicitly. Both MCP service and outward gateway pass effective caller identity. Existing ownership/atomic consume checks remain; consume also rejects newly expired approval. | State approval test checks wrong/null caller exclusion and two independently approved callers. Initial central state suite passed; two-approved-caller expansion needs final rerun. |
| C4-02 / C4-03 swarm dispatch/lifecycle | New `reserve_run` is an atomic INSERT…SELECT checking current lifecycle, workspace, busy agent, capacity and total-run limit. Coordinator, scheduler, utilization and explicit/meta producers use it. Scheduled cursor advances only after reservation. A weak per-swarm operation gate serializes coordinator/lifecycle, manual dispatch and initial session registration; each prompt write rechecks the persisted live row under that gate. TUI/landing waits hold no gate and notice a stopped row within 250 ms. Pause, abort, budget pause, board clear and single-run stop order terminal state before cleanup; manual runs on ordinarily paused swarms remain supported. Meta-agent callbacks/settlement now CAS live states so a late callback cannot resurrect a stopped turn. | Atomic capacity/same-agent/paused/manual/aborted reservation test passed in initial state suite. Server `dispatch_tests::stopping_during_readiness_does_not_wait_or_send_late_input` passed in central server run. Final meta CAS changes require final rebuild. Root review caught the initial overly broad readiness lock; that implementation was replaced before completion. |
| C4-04 / C4-05 workflow launch | Run reserves its guard before validation, captures workflow/workspace/view generation, stops launch after navigation during preflight/save, and installs a POST result only in its original view. | Deferred-response tests for A→B and A→empty during save, duplicate preflight, validation failure release, late POST response; central UI suite passed. |
| C4-06 workflow history | Version reads own workflow plus request/view generations. Restore binds the version's `workflow_id` before confirmation and checks ownership before dispatch/install. | Deferred A/B history and navigation during restore confirmation tests passed. |
| P4 pool | Two reusable slots are admission capacity; no one-shot fallback under saturation. Health uses the same cached client/slots. Cache insertion double-checks concurrent construction and retains busy clients. Live transports additionally hold one of 64 global permits, released on failure/cancellation/drop. Canceled operations take their transport out of the parked slot, preventing unread replies from poisoning reuse. | Eight concurrent slow calls stay at two server starts; initial MCP suite passed. Later service-health/cache/global-permit changes need focused rerun and N=1/2/8/20 CPU/RSS measurement. |
| P4 buffering | HTTP content-length is checked early; chunks are checked before append. Stdio uses bounded fill_buf/consume across newline-free output, notifications and response lines. Overflow discards the transport. | Newline-free ongoing producer and cumulative notification-budget tests passed. Separate chunked HTTP/SSE fixture and concurrent RSS measurement remain central verification gaps. |
| P4 shell | stdout/stderr drain concurrently, retain 512 KiB each, append omitted-byte counts and continue discarding until EOF. Timeout/process-group/exit behavior remains. | Deterministic producer exceeds both pipe caps, preserves exit 7 and reports truncation. Central server tests passed except unrelated new batch harness limit below; existing timeout/process-group regression included. |
| P4 capabilities | Bounded `/access/{kind}/{id}/capabilities/batch` returns ordered child decisions; shared groups, policy and operation/page/legacy ceilings load once per request. Pure deny-wins evaluator remains authoritative. Tools catalogue retains mounted child batches, refreshes by batch and releases unused entries/in-flight installs when it leaves. Membership uses Sets to avoid quadratic refresh filtering. Catalogue reads reject late server responses. | UI 1000-tool load/refresh/release uses one batch per refresh and zero per-tool calls; passed. Backend allow/deny, root, disabled account and 1000/1001 boundary test added. Its first central failure was the old *test harness* 1 MiB read cap; helper now allows 8 MiB for batch responses only, asserts actual size and compares every entry with the single-item endpoint. Rerun pending. Contract and TS request type updated together. |
| P4 usage | Mission Control refresh collects only session/external-trigger source IDs from its workspace's bounded 500 items and uses one lifetime GROUP BY query. Missing usage stays absent; unchanged cost does not write; zero totals can correct a stale cost. No dashboard date or row window substitutes for lifetime totals. | Source/consumer check pending central usage build; a 500-session query-count/usage-data integration fixture remains unexecuted. |
| UX4-01 policy replacement | Replacement fetches all policies before confirmation even when current workspace is empty. Confirmation discloses instance-wide deletion, total rules, global rules and number of affected workspaces. Failed scope fetch prevents POST. | Global-only-other-workspace, cancel and failed-scope-fetch tests passed. |
| UX4-02 Mission Control drafts | Detail exposes actual-field dirty/leave decision and router guard. Selection/close await it before mutating selected ID or URL; cancel retains draft. Save in flight blocks leave. Detail read success/error/loading are workspace-owned. | Production canLeave/select methods tested for Keep editing and Discard with route/draft assertions; passed. Browser list/graph/close/module-navigation cases remain for E2E. External mobile Drawer changes still belong to Claude. |
| UX4-03 Loop partial success | Extension remains open until Resume succeeds; partial error explains budget saved / not resumed. Retry skips the completed PATCH for unchanged requested limits. Closing is disabled during action; a response after leaving uses a toast. | PATCH-success/Resume-failure/retry regression passed. |
| D4-01 library failure | Library loading/error/success-empty state plus Retry; failed fetch preserves suggestions and typed/selected input. Mount load is untracked to avoid reactive retry loops. | Source inspection complete; delayed/error/recovery browser case pending. |

## Central evidence communicated by parent

- Initial full library run: `otto-mcp` 35 tests passed; `otto-state` 370 passed, 2 ignored. `/tmp/otto-review-orchestration-state-green.log`.
- Combined UI: `npm run check` zero errors/warnings; full unit suite 1,062 passed, including `ui/unit/orchestrationOwnership.test.ts`. Last subsequent UI change only replaces catalogue membership `.includes` with Set `.has`; rerun that focused suite after integration.
- Combined Rust run: design 123 passed; MCP 35 passed; server 1,282 passed, one new fixture failure. `/tmp/otto-review-wave2-rust-libs.log`. The failure was `LengthLimitError` in the batch test harness, not an incorrect allow/deny decision. Fixture correction is authored; final gate pending.
- Owned Rust `git diff --check` passed before the final narrow changes. Parent ran rustfmt centrally; latest `client.rs`, `service.rs`, `swarm_agent_run.rs`, `mcp_control.rs`, and `resource_access_tests.rs` edits may need one final formatting pass.

## Next exact checks

Parent runs one heavy command at a time:

```sh
cargo test -p otto-server --lib capabilities_batch_preserves_child_denials_and_bounds -- --nocapture
cargo test -p otto-server --lib dispatch_tests
cargo test -p otto-server --lib scheduled_tasks_engine
cargo test -p otto-mcp --lib
cargo test -p otto-state --lib mcp_control
cargo check -p otto-server --lib
cd ui
node --test unit/orchestrationOwnership.test.ts
npm run check
```

The normal scoped `scripts/check.sh --check` remains the final integration gate. Root should record actual capability response bytes from the boundary test, query counts/SQL invariants, CPU/RSS at MCP N=1/2/8/20 and scheduled output N=1/2, real-app read-only resource samples, plus light/dark/mobile evidence. No runtime/performance/screenshot success is claimed here.

No finding was rejected. Implementations are ready for independent iteration-2 review, with the explicit outstanding verification above.
