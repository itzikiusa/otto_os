# Full round 2 — performance

**Verdict: Approve the inspected browser resource repairs.** Confirmed open findings: **0 blockers, 0 major, 0 minor, 0 nits**. Reviewer 10. Baseline `7cff9e538deccdd0e346af7c6285e11863285c00`; reviewed branch checkpoint `195de62c8` plus this round's uncommitted test additions. No new production fix was justified. This review closes a specific real-Chromium protocol gap and does not relabel earlier reviewers as additional full rounds.

**Product assessment:** **9.8/10 for the inspected browser resource-admission and responsive-rendering scope**, an engineering judgment rather than a measured quality percentage. **Confidence: medium-high** in the bounded admission behavior and measured protocol envelope; medium for general browser performance. This is not a whole-product score. The remaining 0.2 deduction is the concrete positive-network validation task below. Unmeasured native/terminal/all-module workloads are confidence/scope boundaries, not invented product defects. The old 9.2 assessment mixed missing runtime evidence with product deductions; this report separates those explicitly. Tests do not themselves make production code faster.

## Scope, sized paths and source findings

Read the complete changed browser transport/process/proxy/session hunks and surrounding launch, session input/navigation, screencast, guard cache, runtime admission and lifecycle. Ran the performance skill's hotspot scan against the pinned baseline; retained [hints](../evidence/round-2-performance/hotspots.log), without treating grep output as findings. Four resource tiers were traced:

| Tier / source | Cost bound and preserved behavior |
|---|---|
| CDP calls, `conn.rs:209` | At most 256 unanswered call waiters. Cancellation removes its waiter; a draining browser cannot retain request-rate × 30 seconds of waiters without admission. |
| CDP queues, `conn.rs:32`, `:250`, `:339` | 256 messages per channel; separate 32 MiB output charge and shared 32 MiB retained-event charge across routes/guard workers. Overload closes the connection and settles waiters. Charges are conservative accounting, not exact RSS. One current inbound message may still reach 96 MiB. |
| Guard work, `process.rs:321` | At most 64 admitted workers, acquired before task creation. Fresh positive safe-method cache hits continue inline; unknown/expired/negative/outward cases retain bounded guarded handling. DNS vetting has a 15-second timeout. |
| Proxy work, `proxy.rs:53` | At most 128 accepted socket tasks, owned by a JoinSet and aborted with the owner. Overload closes excess sockets before spawning work. |

These caps compose with four maximum Chrome processes, serialized session admission and a configurable 1–16 session limit (`runtime.rs:302`, `:325`, `:408`; `types.rs:13`, `:107`). The SOCKS proxy is shared across those processes. A viewer retains two unacknowledged frames plus only the newest pending frame (`flow.rs:15`, `:65`); its message channel has 64 slots. Frame bytes are shared by Arc rather than copied per viewer. These are source bounds, not a promise that total Chrome renderer memory is fixed.

The scale question is now fixed admitted work versus arrival rate, rather than arrival rate × residence time retained without admission. Sustained excess traffic can still terminate the transport by design; this pass found no evidence of an ordinary measured burst triggering that overload policy. The existing cached-positive 128-request regression remains necessary: the real-browser workload below checks denied HTTP traffic, not that positive HTTP cache path.

Broader hot-path sampling: transcript initial/offline folds share two workers, eight pending folds, eight waiters per identity and a 128 MiB offline cache (`transcript_cache.rs:21–35`); live tails have a 64-entry ceiling (`transcript_tail.rs:38`). Session WebSocket input's nominally unbounded channel is charged to an admission semaphore (`ws.rs:1495`), so its channel type alone is not an unbounded-memory finding. Holder input/ACK code was sampled, but no new claim about its entire caller graph is made. The previously measured terminal workload remains historical evidence from a different binary/Node runtime, not a fresh result. Native work belongs to the concurrent reviewer.

## Real Chromium acceptance and provenance

Added [`session_chromium_tests.rs`](../../../../crates/otto-browser/src/live/session_chromium_tests.rs) and its test-only registration at `session.rs:1741`. No production behavior, DTO, security default, or network policy changed. The ignored opt-in test launches the actual production `ChromeProcess`, `LiveSession`, `CdpConn`, `GuardProxy` and viewer flow. It uses a fresh temporary profile, sandbox enabled, CDP on pipes, no debug port and no real daemon/user workspace. A fixture page is installed directly into the initially blank target for the test; this is not an assertion that the public navigation API accepts data URLs.

Chrome for Testing **149.0.7827.55**, installed revision 1228; browser-level Fetch supported. Rust **1.99.0**, debug test profile, four Tokio workers. Host Mac15,9, macOS 27.0.1. Node **26.10.0** is recorded for campaign consistency, but **no Node worker participates in this Rust workload**. Exact binary and source hashes: [provenance](../evidence/round-2-performance/provenance.json). The test links current browser source directly; it neither relies on nor measures the rebuilt full-daemon binary.

Each of 12 rounds rotates through 1, 32 and 128 resources. It verifies real `Input.insertText` through the production input dispatcher and reads the resulting input value; decodes that many inline SVG images; starts that many HTTP fetches to a fixture-owned loopback listener; requires every failure to be explicitly `blockedReason=inspector` / `ERR_BLOCKED_BY_CLIENT…` in a separate CDP Network observer; makes eight same-document history changes; changes visible text and requires a new screencast frame. Every received binary frame is parsed and ACKed through the actual viewer interface. A two-second recovery follows each round. Final screenshot, session unregister, process/connection shutdown and untouched forbidden listener are asserted.

**Acceptance met:** 644/644 decoded images, 644/644 requests guard-blocked, 96 SPA history changes, 12 verified text inputs, **87 screencast frames**, final 8,070-byte PNG, live connection/viewer after every round, and no connection accepted by the forbidden listener. Completed in **25.84 seconds**. All owned sampled PIDs were absent after teardown; all four profiles from functional/diagnostic/measured runs were removed. [Cleanup](../evidence/round-2-performance/cleanup.json).

The first functional run and one diagnostic rerun failed only because the test assumed the error text lacked Chrome's `.Inspector` suffix. The diagnostic captured the actual error and `blockedReason`; the corrected assertion still requires explicit inspector blocking. Those are retained harness failures, **not** production red/green evidence. The corrected functional and measured runs both passed. No resource/time threshold or production guard was relaxed.

## Measured cost

The measured run held the exclusive campaign runtime lease: other campaign Cargo/browser/native work was paused. Ordinary user applications were not stopped. The [sampler](../evidence/round-2-performance/sample.py) retained 46 process-tree samples, nominally 0.5 seconds apart plus `ps` overhead. It records only the launched test and its descendants. [Raw samples](../evidence/round-2-performance/resources.json), [summary](../evidence/round-2-performance/summary.json), [execution log](../evidence/round-2-performance/chromium-run.log).

| Resource count per round | Four samples: mean ± population SD | Min–max |
|---|---:|---:|
| 1 | 82.82 ± 6.73 ms | 72.26–90.87 ms |
| 32 | 88.76 ± 11.92 ms | 74.06–100.53 ms |
| 128 | 87.40 ± 3.89 ms | 83.23–93.23 ms |

This duration covers input verification, image decode, blocked fetch completion, eight history updates and observing a subsequent frame; it excludes the two-second recovery. Four samples per size do not establish a population p95. No latency cliff was observed at 128 resources. The test's progress deadlines are 15 seconds for evaluate and five seconds for guard/frame completion; the observed sub-101 ms times are observations, not newly imposed universal SLAs.

Rust test-host RSS peaked at **13.59 MiB**. Summed Chrome-plus-host RSS peaked at **1,561.94 MiB**; summing process RSS can count shared pages multiple times and is not unique physical memory. After the first six seconds, summed RSS ranged **1,550.59–1,561.94 MiB**; `ps` CPU averaged **4.23%**, max **36.0%** (100% = one core). Startup's sampled CPU maximum was 158.4%. The host is the browser-crate test process, **not the full daemon**, and the workload includes the extra Network observer. Small retained growth across 26 seconds does not establish either a leak or leak freedom. The important transport observation is continued work and reclaimed teardown, not an invented RSS ceiling.

## Executed gates

- `cargo test -p otto-browser --lib --no-run --message-format=json`: compiled the opt-in test. [Compile log](../evidence/round-2-performance/compile.log).
- `cargo test -p otto-browser`: **127 unit + 3 integration tests passed**, one opt-in test ignored by default, zero failures; zero doctests. Existing burst/budget/proxy saturation regressions are included. [Log](../evidence/round-2-performance/browser-tests.log).
- `cargo clippy -p otto-browser --all-targets -- -D warnings`: passed. [Log](../evidence/round-2-performance/clippy.log).
- Named opt-in functional command: `OTTO_TEST_CHROME_BIN='<pinned Chrome executable>' cargo test -p otto-browser --lib real_chromium_resource_bursts_keep_navigation_and_frames_live -- --ignored --nocapture`: passed after correcting the harness protocol expectation.
- Measured execution: `OTTO_TEST_CHROME_BIN='<pinned Chrome executable>' python3 docs/reviews/quality-next-20261008/evidence/round-2-performance/sample.py /Users/itziklavon/otto_os/target/debug/deps/otto_browser-1194e6b985037524`. The sampler records the exact direct-test command in raw samples. No compilation overlapped measurement.
- Owned Rust source formatted; owned source/report whitespace check passed. Coordinator owns final workspace integration gates.

## Remaining deduction and bounded next task

**P-V1, 0.2 points: positive HTTP resource burst through the actual guard/proxy.** The prior 128-request cached-positive test exercises the real pipe and guard dispatcher with a controlled CDP peer, while this real-Chrome test exercises decoded inline resources and blocked HTTP requests. Together they still do not prove successful HTTP delivery through Chrome's real connection pool/proxy at the admission boundary.

Concrete completion: use an explicitly owned, public test origin (or a test-only dial fixture that preserves the production guard decision); warm its origin with one document; load 128 small uncached assets for three rounds; require all 384 expected bodies, zero guard-overload rejection, a progressing input/screencast after each round, and closed fixture-owned sockets/processes on teardown. Cap the workload and retain command/source provenance. Do not blast a third-party site or disable the production guard to manufacture a passing test. This is a bounded validation task, not a claim of a discovered defect and not a reason to demand an undefined hours-long soak.

No whole-product 9.8 claim follows from this report. Native composition, the full shell, real agent providers and combined cross-module contention are outside this pass; they reduce generalization confidence rather than adding arbitrary performance deductions.
