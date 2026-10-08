# Independent browser/UI review — iteration 2

**Verdict: approve the inspected repair, conditional on coordinated green validation; one major scheduler-sensitive admission finding reproduced and repaired in source.** Reviewed baseline `7cff9e538deccdd0e346af7c6285e11863285c00` and the shared working-tree browser/UI repairs on `review/quality-20261008-next`. This report is an independent scoped assessment, not certification of the entire application.

## Finding IBUI-01 — cached public resource bursts can be rejected before guard workers run

**Major; confirmed by source trace and executed deterministic positive regression.** `crates/otto-browser/src/live/process.rs:325` acquires one of 64 permits synchronously; `:337` spawns the worker that ultimately releases it. The browser event pump at `:613` and page event pump in `session.rs:1599` can consume already-buffered events without waiting for those workers. With 128 ordinary GET image requests for an origin already in the verdict cache, and no worker polled during delivery, requests 1–64 consume all slots; requests 65–128 receive `Fetch.failRequest` with `BlockedByClient`. None needs slow DNS or an outward approval. Whether this scheduling sequence occurs for a particular real page remains to be measured; this is not a claim that every 128-resource navigation fails.

The repair's intended bounded admission is necessary, but instantaneous scheduling saturation is not evidence that these requests are unsafe. A cached-positive GET/HEAD/OPTIONS fast path using the existing origin key and TTL fixes this concrete case without expanding any limits. All unknown/negative verdicts and state-changing methods must retain the existing guard/approval path. An ordinary initial Document is vetted before same-origin subresources begin; many cached subresources therefore need no worker. This does not promise uncached bursts above 64 will pass. A bounded waiting tier is another possible design if later practical workloads justify it; unlimited semaphore-waiting tasks must not return.

Cost model: active work stays at 64; a queue of Q reduced/charged events adds O(Q) fixed retained work rather than O(arrival-rate × DNS-delay) unbounded tasks. Retaining the existing event lease prevents the queue from escaping the 32 MiB accounting budget. The positive fixture `process_burst_tests.rs` feeds 128 real CDP pipe frames, preloads the public-origin verdict, delivers decoded events to the production synchronous handler, and expects 128 continuation commands and zero rejection commands. It is deliberately scheduler-controlled and does not substitute for real Chromium.

**Repair review:** the coordinator implemented the cached-safe fast path at `process.rs:331`. I independently reread it: it preserves the same origin key and TTL, requires `Some(true)`, restricts inline continuation to GET/HEAD/OPTIONS through `is_outward_method`, preserves `event.session_id`, and leaves all other requests on the previous guard/approval path. No limits were raised. This resolves the traced defect at source level. The positive test now also asserts session routing for all 128 outputs. A second test covers cached-negative, expired-at-TTL, uncached, POST and DELETE cases with occupied workers; none may bypass admission. Coordinated green execution remains pending at this report's handoff.

## Independent trace coverage

- Read all five changed browser files and all three changed UI files, their relevant callers, tests and contract changes. Scoped each queue and lock before treating it as a concern.
- Transport: command cancellation drops pending entries; explicit shutdown fails pending callers and signals closure; writer failure wakes the reader; event overload closes rather than silently dropping guarded requests; a closed per-session receiver drops late events; route mutation locks do not span async work. Per-process output/event accounting is bounded separately, with 256 pending calls. The 96 MiB inbound wire-frame limit and JSON decode are outside the retained-event accounting bound, and result replies are not covered by the event lease. No exact RSS bound is asserted.
- Process: guard admission happens before spawning and the original event charge survives guard work; connection closure cancels admitted workers. The positive-burst finding above is separate from the valid overload safety fix.
- Proxy: one runtime-wide proxy has 128 accepted workers across all Chromium processes; JoinSet ownership aborts accepted tasks with the listener, completed workers reclaim admission. DNS has a deadline. Established-tunnel/native-browser teardown was not exercised independently.
- UI: toolbar focus restoration waits for Svelte rendering and refuses to steal unrelated editor focus; fallback candidates skip disabled actions. Workspace error publication checks generation and token, while the underlying workspace store owns list/selection generations. Proof's new workspace-readiness dependency avoids the cold-link request being invalidated by the later store reset. Existing errors retain an explicit Retry path. No additional confirmed changed-line UI finding.
- Adjacent breadth samples: API request execution snapshots draft/environment and retains controller/tab ownership through pre-script, confirmation, HTTP and post-script; DB query execution checks access epoch, controller and pending query identity across guarded-write confirmation; Git primary status/PR requests use independent generations and current repo ownership. These samples do not certify every API/DB/Git operation or server authorization boundary.

## Design and usability evidence

Read the design guidelines README, review checklist and accessibility guidance. Independently opened the existing iteration-1 captures for API phone light, API desktop dark, and workspace Retry on phone. The inspected captures show visible toolbar focus, readable hierarchy, a reachable recovery control and no visible clipping. They are another reviewer's captures, not freshly executed screenshots. Source review of the new Playwright cases verifies that they check focus transfer, actual overflow action activation, preserved URL draft, retry recovery and stale failure suppression, rather than only element presence.

## Scoped four-vertical assessment

| Vertical | Assessment | Evidence and limit |
|---|---:|---|
| Performance | 9.2/10 | Fixed retained-work bounds and the source-reviewed cached fast path address the inspected cliffs; no independent RSS/CPU/native soak was run. |
| Correctness | 9.1/10 | Cleanup and ownership traces are sound in the inspected paths; the independently reproduced defect is repaired in source, with green regression execution pending. |
| Design | 9.2/10 | Shared chrome/recovery captures and source follow the design rules; only three screenshots and the changed shared surfaces were inspected. |
| UX/usability | 9.2/10 | Focus, recovery and cold-link changes preserve user intent; the source-reviewed fast path addresses the reproduced browser rejection. Native and populated whole-product journeys remain unverified here. |

Scores are independent engineering judgments for this slice, not pass percentages. The requested whole-product 9.8 is not established by this review. Evidence gaps are not asserted as product defects.

## Verification and ownership

The performance hotspot script was run against the pinned base; it examines committed changes and printed no changed files because this repair set is unstaged. It therefore supplied no useful coverage; actual working-tree diffs were read directly. `git diff --check` passed at the inspected checkpoint.

With the coordinator's Cargo lease, ran `cargo test -p otto-browser --lib cached_public_resource_burst_does_not_fail_before_workers_are_polled -- --nocapture`: **expected red, 0 passed / 1 failed**. The assertion recorded actual `(64, 64)` continuations/rejections versus expected `(128, 0)`; compile took 1.93 s and test 0.01 s. Durable log: [burst-red.log](evidence/independent-browser-ui/burst-red.log); source checkpoint: [reviewed-source-sha256.txt](evidence/independent-browser-ui/reviewed-source-sha256.txt). Cargo was then explicitly released to the correctness reviewer. Root owns the production repair and subsequent green validation.

No browser, daemon or production operation was executed by this reviewer. Only this report/evidence and the explicitly authorized isolated regression file were created; production code was not changed by this reviewer. The coordinator attached the test module in `process.rs`.
