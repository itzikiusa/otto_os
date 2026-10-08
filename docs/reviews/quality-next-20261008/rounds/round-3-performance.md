# Full round 3 — performance

**Verdict: Approve.** Reviewer 13/20; baseline for this pass `d4b8626d9` plus the source hashes in this report. One minor failure-history growth issue was reproduced and repaired. **Open findings: 0 blockers, 0 major, 0 minor, 0 nits.** Round 2's concrete positive-network task P-V1 is now closed.

**Performance assessment: ≥9.8/10 for the inspected browser resource-admission, failure-history and responsive-rendering scope.** This is an engineering assessment against the declared acceptance tasks, not a measured percentage or a whole-product claim. No unresolved product deficiency is being hidden in a score deduction. Confidence is high for the deterministic bounds/regressions and medium-high for this real-Chromium workload; Internet latency, TLS/HTTP2 pooling, other operating systems and combined whole-app contention remain generalization boundaries. They are not fabricated defects or reasons to invent an undefined soak requirement.

## Sized source review

Read the changed browser CDP transport, guard dispatch and SOCKS proxy hunks against `origin/main`, then the surrounding guard cache, session input/navigation, frame flow, process lifecycle and runtime admission. The skill hotspot scan is retained in `evidence/round-3-performance/hotspots.log`; signatures are candidates, not findings.

- Per Chrome process, at most 256 unanswered commands and 256 queued messages per route; separate 32 MiB output and shared 32 MiB retained-event charges. Event charges remain owned while asynchronous guard work uses the event. A current wire message is separately capped at 96 MiB. These bounds are accounting envelopes, not exact RSS.
- Guard admission takes one of 64 permits before spawning. Unknown/negative/expired/outward requests retain bounded vetting; fresh positive safe requests continue inline after an O(1) origin lookup. The independent proxy vets every dial. Consequently an ordinary cached-origin burst does not turn scheduler delay into false 64-worker saturation.
- The SOCKS proxy accepts at most 128 owned socket tasks; dropping its JoinSet aborts accepted tasks. Socket transfer is streaming, not whole-body buffering. Chrome session admission is serialized; the host allows four Chrome processes and at most 16 configured sessions. Excess arrival rate cannot grow these queues without bound.
- Each viewer holds two unacknowledged frames and one replaceable latest frame, with shared frame bytes. No historical-frame accumulation occurs behind a slow viewer.
- Broader source sample: transcript folds share two blocking workers, eight pending folds, eight waiters per identity; retained offline results cap at 32 entries/128 MiB, individual cache entries at 32 MiB. The live-tail registry caps at 64 and reserves per-tail/global byte accounting before retaining a fold. No new performance claim about the full transcript pipeline follows from this sample.

At 10× or 100× offered request rate the implemented policy reaches explicit admission limits; it does not promise lossless overload processing. The important unresolved acceptance from round 2 was whether healthy real Chrome HTTP resource loads at 128 assets succeed under those limits.

## Repaired finding

### [minor] Failed-origin history grows across the lifetime of an active reader — `crates/otto-browser/src/lib.rs:495`

**What:** `BrowserService::record_failure` inserted every distinct failed host into its fallback history. Cooldown checks ignored old timestamps but never removed the entries; only a future success for that same host erased one. A continuously active reader could retain every historical failed origin.

**Cost:** n is the number of distinct origins whose engine attempts returned `Unavailable` during one service lifetime. The old map retained O(n) host strings plus hash-table and `(count, Instant)` overhead indefinitely, even though the fallback window is ten minutes. At 100,000 such hosts this is megabytes of irrelevant retained history; normal interactive use is much smaller, so this is a minor failure-path growth issue, not a demonstrated normal-navigation latency problem. Idle service teardown also limits its lifetime; this is not claimed to grow across daemon restarts.

**Evidence:** Two new regressions failed before repair: 1,025 distinct failed origins left 1,025 entries, and an expired unrelated origin remained present after a fresh failure. The same tests now pass. See [red output](../evidence/round-3-performance/failure-history-red.log) and [complete green browser suite](../evidence/round-3-performance/browser-tests.log).

**Fix/payoff:** Keep at most 1,024 host entries. Existing-host failures update in O(1), preserving the accumulated failure count so an unsuccessful post-cooldown probe immediately rearms fallback. Only inserting a new host prunes expired unrelated entries and, if still full, evicts the oldest failure with one bounded scan. Storage becomes O(min(n, 1,024)); normal lookups and repeated failures stay O(1). A rarely revisited evicted host gets another primary-engine probe rather than permanent fallback; this map is a performance hint, not an authorization policy. Tests verify preserved recent cooldowns and failed-probe rearming as well as capacity and expiry.

## Deterministic network fixture

Added a `cfg(test)`-only, per-proxy exact socket mapping. It runs **after** unchanged production `vet` succeeds. Both the Fetch guard and proxy see `198.20.0.1:8123`, a public literal accepted without DNS; only the final socket dial maps to the fixture's owned ephemeral loopback listener. Every unmapped fixture-mode destination is refused. No packet is sent to that public literal or another third-party origin. The normal non-test build has no mapping parameter or mapping branch.

A mandatory non-ignored regression maps a blocked loopback address and requires SOCKS denial before the listener can be touched; it also sends a different public target and requires refusal instead of external dialing. This seam is a deterministic transport fixture, not an Internet benchmark and not permission to relax production SSRF policy.

The opt-in test launches the actual sandboxed ChromeProcess/LiveSession/CdpConn/GuardProxy with an ephemeral profile and pipe CDP. It loads one real HTTP document through normal navigation to warm the origin, then three bursts of 128 unique uncached requests. It checks every exact response body, all 384 distinct paths at the HTTP listener, zero Network.loadingFailed events, actual dispatched text input, and a fresh parsed/ACKed screencast frame after each burst. The fixture returns `Cache-Control: no-store`, and each URL is unique. It closes each HTTP connection, so all bodies traverse new proxy socket lifecycles rather than only one permanently warm socket.

Chrome retains its real connection pool; 128 issued fetches do not mean 128 simultaneously open upstream sockets. Each round's duration includes HTTP delivery, input verification and a subsequent frame. Three observations cannot establish a population percentile. Shutdown requires session unregister, closed transport, closed fixture/proxy listeners, a removed temporary profile and no remaining sampled owned processes.

## Execution, provenance and reassessment

The initial functional attempt and diagnostic rerun used `8.8.8.8:80`; Chromium synthesized 307 redirects with `Non-Authoritative-Reason: HttpsUpgrades` between HTTP and the intentionally unmapped HTTPS endpoint, ending in `ERR_TOO_MANY_REDIRECTS` before the workload. The fixture served no redirect. The diagnostic log retains those actual Network events. The final fixture uses `198.20.0.1:8123`, and the recorded document response is HTTP 200 without entering that upgrade path. No guard, browser security flag, admission threshold or production navigation behavior was changed to obtain a pass. This is fixture incompatibility evidence, not a claimed repaired production bug. The fixture also tolerates an unused preconnected socket closing without sending a request.

The corrected functional run passed in **7.18 s**; the exclusive measured repeat passed in **6.97 s**. Both delivered **384/384 exact bodies**, zero observed network failures, all three text inputs and new frames after each round, and successful teardown. The measured repeat received **23 parsed and ACKed frames** and a final **9,401-byte PNG**. Measured round durations were **76.36, 63.78 and 63.80 ms**, mean **67.98 ms**, population SD **5.93 ms**. These three samples do not establish a population percentile or an Internet SLA.

The sampler retained 13 process-tree observations. Rust test-host RSS peaked at **13.41 MiB**; summed Chrome-plus-host RSS peaked at **1,572.61 MiB**. Summed RSS can count shared pages more than once. Mean sampled `ps` CPU was **27.38%**, startup-inclusive max **139.0%**, where 100% is one core. This seven-second run proves forward progress and teardown, not long-term leak freedom. Every sampled owned PID was gone after exit. All four temporary profiles from functional, diagnostic, corrected and measured runs were absent; no remaining process command referenced them.

Chrome for Testing **149.0.7827.55**, installed revision 1228; production sandbox enabled, browser-level Fetch interception, pipe CDP. Rust **1.99.0**, debug test profile, four Tokio workers, Mac15,9/macOS 27.0.1. Node 26.10.0 is recorded for campaign consistency but is not involved. This directly linked browser-crate workload does not use the full daemon. The measured test ran under the exclusive campaign runtime lease with other Cargo/browser/native work paused; ordinary user applications were not stopped.

Retained evidence:

- [Exact source, test executable and Chrome hashes](../evidence/round-3-performance/provenance.json).
- [Initial failed fixture](../evidence/round-3-performance/chromium-functional.log), [Network diagnostic](../evidence/round-3-performance/chromium-diagnostic.log), [corrected functional pass](../evidence/round-3-performance/chromium-functional-corrected.log).
- [Measured execution](../evidence/round-3-performance/chromium-run.log), [raw resource samples](../evidence/round-3-performance/resources.json), [summary](../evidence/round-3-performance/summary.json), [cleanup](../evidence/round-3-performance/cleanup.json), [sampler](../evidence/round-3-performance/sample.py).

Final-source gates:

- `cargo test -p otto-browser`: **130 unit + 3 integration passed**, two opt-in real-Chrome tests ignored by default, zero doctests. Includes the new mapping-policy regression, failure-history repair regressions, existing retry-window tests and transport/cache/proxy saturation tests. [Output](../evidence/round-3-performance/browser-tests.log).
- `cargo clippy -p otto-browser --all-targets -- -D warnings`: passed. [Output](../evidence/round-3-performance/clippy.log).
- Functional command: `OTTO_TEST_CHROME_BIN='<pinned executable>' target/debug/deps/otto_browser-1194e6b985037524 --ignored --exact live::session::http_chromium_tests::real_chromium_positive_http_bursts_preserve_bodies_input_and_frames --nocapture`: corrected run passed.
- Measurement: `OTTO_TEST_CHROME_BIN='<pinned executable>' /opt/homebrew/bin/python3.13 docs/reviews/quality-next-20261008/evidence/round-3-performance/sample.py /Users/itziklavon/otto_os/target/debug/deps/otto_browser-1194e6b985037524`: passed; exact expanded command is in raw resources.
- Owned Rust formatting and source/report whitespace checks passed. Coordinator owns workspace/consumer integration gates.

The source repair bounds historical failure state; it does not claim to speed up successful navigation. The new real-browser acceptance closes a specifically identified validation gap. No outstanding performance defect remains within this pass's declared scope.
