# Independent responsiveness plan reviews

All required amendments were accepted and recorded in the companion implementation plan before its execution gate.

# Independent plan review: Agents and Workflows

Status: approved subject to the explicit ordinal/import amendment below being recorded before implementation. Reviewed all nine tasks against the binding design addendum and the current direct-archive writer behavior. Other work is scoped, testable and includes actual call-site lifecycle/authorization/compatibility tests.

Required clarification: stable checkpoint ordinal assignment must cover migration, direct INSERT and archive restore, including omitted and hostile duplicate supplied ordinals. Repository-only allocation would allow an entire raw-imported page to share a cursor position and skip records. Recompute derived ordering transactionally, keep authoritative bodies exact, and add lossless paging fixtures for multiple direct inserted rows. Projection invalidation/repair must also advance the revision visible to after_rev so imported stale metadata is not hidden behind an unchanged response.

Accepted limits: bounded metadata windows preserve exact Unicode matching with explicit continuation; old History costs remain only on the legacy route. Cache cold-fold and repair admission is bounded while retained warm hits bypass it. Explicit large detail remains possible and live tail arming remains separate from provider resume. No additional feature or persistence redesign is requested.


---

# Independent Vault implementation-plan review

Reviewed `/tmp/otto-perf-plan-vault.md` against amended `/tmp/otto-perf-design-vault.md`. Read-only review; no application edits/tests/builds. Scope: execution correctness, regression quality and avoidable amplification introduced by this plan.

Verdict: approve with two bounded execution clarifications. The original hydration and structural/incomplete-walk blockers are resolved explicitly in Tasks4/8, including deterministic real-boundary regressions. File ownership, migration0131, unchanged source/hash/recovery promises, transaction rollback, and visible-tree response ownership are concrete and testable.

## P2 clarification: do not multiply global link reconciliation by changed file count

Task5 performs global incoming/unresolved resolution for path additions/removals; Task7 prepares and applies bounded batches. A straightforward composition can run an O(E) global link pass for every newly discovered file (or every small batch), making a cold import/large external folder addition approach O(N*E). This is an implementation risk, not a measured current regression.

Amend Task7 with a concrete reconciliation schedule: collect lightweight accepted path-membership changes, build/apply the final resolver membership for the publication unit, and perform at most one global destination pass for that unit; do not call the single-note path-add helper independently N times. Parsed bodies remain streamed/bounded. If large scans publish several units, choose/document the batching tradeoff and instrument the number of global passes so it cannot accidentally scale per file. Add a cold fixture with many new targets and previously unresolved incoming links, asserting exact final ambiguity/backlinks and the explicit pass budget. Preserve the existing scan/API generation fence when membership changes during preparation.

## P2 clarification: bound pending file-preparation work, not only spawned jobs

Task3 acquires a two-worker semaphore before spawn_blocking, which correctly bounds active workers. It does not by itself bound the number of async callers waiting on the semaphore or their retained submitted note buffers. Task7 must avoid constructing one preparation future per scanned file; require a sequential stream or bounded channel (capacity at most a small multiple of two) and only read/retain a body after admission. API writes should either be serialized before preparation by the existing mutation path or have a small explicit admission bound; overload must return retryable busy rather than collect unlimited pending bodies.

Add the pending/admitted count to the deterministic two-worker test: a large synthetic enumeration must retain only the bounded active/pending preparation units, and canceling a queued caller must remove its pending slot without releasing an executing worker's permit. This reinforces the design's stated lack of an unbounded blocking queue without inventing a hard RSS promise for explicitly requested raw-note GET.

## Accepted without additional changes

- Cold hydration owns publication gate through consistent SQL snapshot and off-thread construction; mutation callers cannot recursively acquire it.
- API source/history/index failure semantics retain actual saved source and avoid false success.
- Prefix-aware structural fences include outside rewritten source notes; partial walk cannot remove entries; removal re-stat treats inaccessible/reappeared files conservatively.
- Unicode/case/basename ambiguity, optional FTS, exact4MiB boundary, downgrade/shrink and migration-era oversized rows have explicit tests.
- Tree refresh is immutable and sequence-owned, with bounded visible sibling concurrency and deferred collapsed refresh.
- Exact raw-note response remains explicitly unbounded by this indexing policy; this is documented rather than hidden.


---

# Independent implementation-plan review: Connections/API

**Verdict: approve after one narrow cancellation-cleanup amendment in Task3.** The plan implements the amended design with realistic file ownership, focused RED/GREEN commands, isolated fixtures and no extra feature work. No code or tests were executed during this review.

Reviewed `/tmp/otto-perf-plan-connections-api.md` against `/tmp/otto-perf-design-connections-api.md` and the current isolated worktree source.

## Required amendment: complete old-resource cleanup when a close waiter disappears

Task3 explicitly tests “close canceled midway” and promises that a guard clears the closing state, but its acceptance only requires retiring old work and avoiding permanent closing. The implementation sequence then performs awaited native cancellation before detaching/closing pools and the cached tunnel. A dropped close future during that first await can therefore clear closing while leaving the old ready resources/cache ownership intact indefinitely.

Current anchor: `crates/otto-dbviewer/src/service.rs:678–720` awaits native cancellation before active-key removal and pool/tunnel teardown. This is exactly the sequence the planned lifecycle refactor retains. The proposed caller-owned initialization policy is fine; approved resource teardown has a different completion obligation.

Amend Task3 to make cleanup own its lifetime after retirement (an owned bounded cleanup task, or a guard that continues disposal of precisely the captured old-generation resources). It must not drain fresh-generation resources if a new request is admitted after the closing phase. Preserve native cancellation before shutting down resources it needs; do not solve this by moving all remote work back under a global mutex. Define completion/timeout reporting truthfully: native query cancellation remains best-effort, not a remote rollback guarantee.

Strengthen the cancellation fixture: pause the first native-cancel await, drop the HTTP/waiter future, then release the fixture; assert every old ready pool/cache key/tunnel ownership is removed or explicitly transferred to the cleanup owner, no stale acquisition succeeds, and a later generation's resource survives completion of the old cleanup. A cleared boolean alone is insufficient evidence.

## Accepted points

- Task1 has direct cross-key ordering tests, same-key single-flight, dropped-initializer retry, late-resource disposal and bounded empty-slot retention. The new module serves multiple real drivers and tunnels; this is a justified shared abstraction.
- Tasks2–3 now carry lifecycle authority through resolve, verification, detached execution and native driver acquisition, including the exact close-after-resolve/before-open fixture raised during design review. Existing-resource-only cancellation cannot accidentally initialize a pool.
- Fingerprints, existing credential/access-scope cache keys, Redis logical database separation, native ClickHouse dead-client replacement and held tunnel Arc leases have explicit implementation/test steps. During implementation, make the fingerprint race fixture include an older blocked acquisition released after a newer fingerprint was installed; an ordinary sequential config-change test is weaker.
- Tasks4–5 keep full history and replay/detail intact, materialize compact fields via guarded triggers, and test future direct inserts/old archive shape as well as migration backfill. Summary SELECT avoids body columns. Literal summaries route/policy inventory and workspace denial are covered.
- Task6 tests actual coordinator behavior with deferred transport and scheduling, including stale draft/tab/workspace ownership; it changes both sidebar and compact selectors and coordinates the shared event hook with root.
- Tasks7–8 enforce bounded concurrent stdout/stderr drains and owned-child cleanup, literal staging-path probes, independent authorization cadence, deterministic failure backoff, and unchanged final publication behavior. Fake command tests prove what is sent; retain the accepted primary-source reasoning for OpenSSH glob/brace interpretation rather than claiming the fake executable itself proves OpenSSH parsing.
- Task9 deliberately leaves broad gates, browser fixture execution and the single PR to root. No source changes or live operations are authorized by this review alone.

No further blocker, migration-number collision, missing implementation area or broad scope expansion identified. Apply the cleanup amendment, then proceed through root's implementation gate.


---

# Independent shared-lane implementation-plan review

Reviewed `/tmp/otto-perf-plan-shared.md` against `/tmp/otto-perf-design-shared.md` and the accepted design-review amendments. Read-only plan review; no source changes, builds or test execution.

Verdict: APPROVED. Root amended R1 and the reviewer re-read the updated plan: runner input admission is explicitly before the first postMessage, Worker output admission is before return posting, and oversized-input/DataCloneError/messageerror exactly-once regressions are explicit. No remaining material execution gaps. The original amendment is retained below for review history.

## R1: Make the pre-clone boundary and clone-failure regressions explicit

The third R1 checkbox says the Worker "bounds input before posting" while describing Worker execution. This can be implemented too late: the main thread must validate the 8MiB input admission limit **before the first runner-to-Worker postMessage**. The Worker independently validates the 8MiB result before its return postMessage. Checking the incoming payload only after it arrives in the Worker has already paid the main-thread structured-clone cost, contrary to the accepted design.

Minimal plan amendment: put input admission in the runner checkbox, keep output admission in the Worker checkbox, and add explicit runner tests for (1) oversized input creates/sends no work, (2) synchronous postMessage/DataCloneError, and (3) messageerror. Each error must reject once, terminate the owned Worker when created, clear its deadline/listeners, and never dispatch HTTP after failed pre-script. The existing timeout/late-message/construction-failure tests do not exercise these separate paths. An oversized returned payload should likewise be rejected inside the Worker before return messaging.

This is an execution/test precision amendment; no architecture change or extra subsystem is needed.

## Accepted execution coverage

- R1 spans controller ownership from before pre-script through post-script, rejects main-thread fallback, and confines infinite-loop fixtures to real browser Workers. The budgets are responsiveness limits, not a claim of sandboxing or a hard JavaScript heap limit.
- R2 carries admission permits into actual blocking jobs, explicitly tests cancellation without prematurely releasing capacity, preserves full listing/filter scope and authorization, and covers the virtualized keyboard/active-descendant behavior.
- R3 accounts for global and per-list admission, the overall phase deadline, owned-process cleanup, unknown-state propagation, and the HTTP forced-removal gate. It does not equate unknown with clean or bypass Git’s normal removal protections.
- R4 explicitly audits isolated-browser setup/teardown, pins the test daemon binary and bounds process ownership; it does not deploy or restart the installed app. Final review and exact-head CI/admin merge are staged after implementation and validation.
