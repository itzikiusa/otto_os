# Full round 4 — performance

**Verdict: Approve.** Reviewer 17/20. Starting commit `2913a7d17` plus the exact source hashes recorded below. **Open findings: 0 blockers, 0 major, 0 minor, 0 nits.** This pass independently rechecks the browser repairs and broadens into terminal input/output, retained emulator state, transcript fold admission and client recovery.

**Performance assessment: 9.8/10 for the reviewed browser/terminal/transcript admission and retained-state scope.** No confirmed performance deficiency remains in this pass; there is no undefined score deduction demanding arbitrary additional work. Confidence is high for reviewed bounds and targeted regressions, medium-high for this particular rendered workload. This is an engineering acceptance score, not a measured whole-product percentage or a universal latency guarantee.

## Source review and sized paths

- Browser fallback history remains bounded at 1,024 origins. Existing-host failures update in O(1); only admission of a new failed origin scans at most 1,024 entries to expire/evict hints. Cooldown rearming keeps the existing host's accumulated failures. The round-3 socket fixture is test-only and substitutes its owned endpoint after normal vetting, with unmapped targets refused. Neither production guard policy nor admission capacity was relaxed.
- Terminal output uses a negotiated credit window: normal UI requests 1 MiB, server clamps proposals to 64 KiB–8 MiB. A slow viewer retains at most one additional window server-side before replacing stale output with a snapshot. The stalled-client timer does not fabricate acknowledgements or reopen the window. Shared `Bytes` preserve the common fast path without copying each frame per viewer. See `crates/otto-sessions/src/ws.rs:443`.
- The UI feeds xterm at most two 64 KiB slices concurrently; queued credited live output is bounded by the negotiated window. Snapshot data has separate grid/history bounds. One resize compact runs per window; eligibility filters hidden/offscreen panes, cancellation drops obsolete requests, and a missing reply frees its slot after five seconds. Scrollback is 2,000 rows in embedded panes and 4,000 in primary panes. The parking lot caps at 12 entries and an estimated 48 MiB, expires after five minutes and retains one oversized newly parked item by design. These accounting figures are not exact RSS.
- Terminal input uses a 1 MiB byte reservation and a separate 1 MiB deferred socket queue, then TCP backpressure. Consecutive input writes coalesce up to 64 KiB. A single oversized frame is admitted only alone rather than claiming that the byte reservation bounds that individual frame. The held-PTY client's writer sends at most a 1 MiB chunk and waits for its ACK before sending another; the holder's unbounded channel type is therefore not evidence of an unbounded queue in the production producer path. See `crates/otto-sessions/src/ws.rs:1312`, `crates/otto-pty/src/held.rs:289`.
- Snapshot capture/formatting uses four shared blocking workers; copying the emulator happens under its lock, formatting after release. The PTY byte ring caps at 10,000 lines/2 MiB and its broadcast retains at most 1,024 chunks, each read no larger than 8 KiB. Emulator history is separately capped and shrinks for unviewed sessions. Cost grows with retained rows × columns, not lifetime emitted output.
- Transcript offline folds share two workers, eight pending identities and eight waiters per identity. Cancellation of a caller does not release the worker early. Cache retention caps at 32 entries/128 MiB, rejects entries over 32 MiB, and sweeps after idle expiry without requiring another request. File identity/stamps invalidate replaced or changed transcripts while simultaneous readers reuse one fold.
- Live tails cap at 64 registrations, eight MiB input/16,384 records per incremental tail, 32 MiB per-tail charge and 128 MiB total retained charge. Charging occurs before retaining/folding incoming data. A retired oversized tail emits metadata invalidations no faster than every 15 seconds. Normal tails poll at 700 ms and incrementally fold new records; they do not refold unchanged history on each poll.
- Client transcript recovery coalesces repeated invalidations into at most two in-flight reads and one trailing request per source, aborts inactive generations and ignores hidden views. Live-following history trims at 600 turns or eight MiB; intentionally paged-back history is kept until the reader returns to live. Inactive conversations cap at 24/32 MiB estimated payload; subagent bodies reserve shared capacity before fetching, at eight bodies/32 MiB. These bounds concern retained caches/active-follow mode, not a claim that all user-requested historical pages have a single hard heap cap.

At ten or one hundred times the output/event rate, these paths reach byte/worker admission or coalesce into bounded snapshots/recovery; they do not retain all historical output. More concurrent user-visible terminals still have linear per-terminal emulator/render costs. The runtime workload checks actual 1/3/5-pane progress and quiet recovery rather than extrapolating a universal concurrency SLA.

No new confirmed production performance defect emerged from these paths. In particular, neither small bounded scans nor explicitly paged historical reading justify speculative optimization. The hotspot scan is retained as candidate evidence, not findings.

## Verification

- Targeted existing `termFlow`, `termCompactQueue` and `transcriptLifecycle` tests: **58 passed**, zero skipped, Node 26.10.0. [Output](../evidence/round-4-performance/ui-bounds.log).
- Existing parking lifecycle/budget tests: **9 passed**, zero skipped. [Output](../evidence/round-4-performance/parking-tests.log).
- Server transcript cache/tail filter: **32 passed**, zero ignored. [Output](../evidence/round-4-performance/transcript-tests.log).
- Existing real WebKit rendered-terminal workload: **1 passed**, zero retries/skips; all three 1/3/5-terminal phases completed. [Execution](../evidence/round-4-performance/run.log), [Playwright result](../evidence/round-4-performance/playwright-results.json).


## Actual rendered workload and reassessment

The unchanged `desktop-terminal-rendered-load-perf.spec.ts` ran once on the fresh debug daemon built after round-4 correctness repairs, SHA-256 `14335b30342d5ab8603e9716620d545778a20be7c86775e65dfa1463c00f8dd9`. Node **26.10.0**, Rust **1.99.0**, Playwright **1.63.0**, installed WebKit revision **2359**, Mac15,9/macOS 27.0.1. All nine terminal instances selected **WebGL**. [Source/build/browser provenance](../evidence/round-4-performance/provenance.json).

Each phase configured 90 one-second input sampling intervals and ten quiet intervals. Process sampling adds overhead, so actual combined phase durations were **106.95, 106.96 and 107.08 seconds**, with 95 paste markers per terminal. The test completed in **5.4 minutes**, 5.7 minutes including setup/teardown. It uses real clipboard input through Terminal → WebSocket → daemon → owned cat PTY → xterm parser/render observer. Only the provider CLI is substituted; terminal sockets and rendering are real. Each payload is 861 bytes, approximately one KiB/second per terminal. The full sequence delivered **855/855 markers**, **18 observed credit ACKs**, no missed input ticks and no fatal UI errors.

| Concurrent terminals | Rendered markers | Input-to-render p50 / p95 / max | Peak pending bytes per terminal | Peak queued bytes | Queues after quiet recovery |
| --- | --- | --- | --- | --- | --- |
| 1 | 95/95 | 17 / 26 / 29 ms | 1,273 | 0 | 0 |
| 3 | 285/285 | 22 / 34 / 38 ms | 1,013 | 0 | 0 |
| 5 | 475/475 | 26 / 36 / 40 ms | 1,023 | 0 | 0 |

These are nearest-rank descriptive percentiles of this run, not population guarantees. Per-terminal values and variability remain in the raw/summary artifacts. Frame p95 was **19 ms** in every phase; maximum observed frame gaps were **26/27/26 ms**. The existing timing gate is one second per terminal p95 and was not relaxed.

| Concurrent terminals | Mean summed process CPU during input | Peak summed RSS during input | Mean summed CPU during quiet recovery | End-of-recovery summed RSS |
| --- | --- | --- | --- | --- |
| 1 | 13.97% | 811.77 MiB | 10.03% | 811.05 MiB |
| 3 | 19.43% | 925.66 MiB | 11.85% | 925.92 MiB |
| 5 | 22.57% | 1,191.88 MiB | 13.25% | 1,030.28 MiB |

The sampler includes the isolated daemon, owned agent children, ClickHouse, any sampled collector, and WebKit helpers identified by their owned resource coalition. It excludes the load-driver from totals. **100% CPU is one core**; `ps` estimates are not exclusive operation CPU, and summed RSS may count shared pages more than once. The transient collector is present in the five-terminal peak, so that peak is not attributed entirely to terminal scaling. Browser RSS peaks were 466.84/513.58/547.80 MiB; daemon peaks were 163.78/167.36/183.00 MiB. Recovery keeps terminals and the harness's frame observer mounted; these CPU figures are not a measurement of an idle or hidden application. A bounded run cannot establish long-duration leak freedom.

All 25 sampled owned PIDs disappeared after the fixture's normal teardown, the temporary data directory was removed, and both dedicated listeners **7896/5296** were gone. No user process, running app or port 7700 was mutated. [Read-only cleanup audit](../evidence/round-4-performance/cleanup.json).

The run held the exclusive campaign performance lease: no other reviewer Cargo/build/browser workload ran concurrently. Ordinary user applications were not stopped. The plaintext-secret warning describes the isolated fixture's explicitly selected temporary file backend; provider-ID warnings are expected for the cat shim, which emits no Codex transcript. Neither warning was suppressed or treated as a production repair.

This closes the earlier terminal measurement's exact-version limitation: the current stable Rust/source and Node26 now execute the real workload successfully. It does not claim to measure native Tauri WKWebView, real provider inference, transcript fold latency, Internet navigation, large database grids or simultaneous cross-module contention. Those are generalization limits, not invented product findings or open arbitrary score deductions.

Raw data: [rendered workload](../evidence/round-4-performance/rendered-terminal-load.json), [summary](../evidence/round-4-performance/summary.json), [summarizer](../evidence/round-4-performance/summarize.py), [hotspot candidates](../evidence/round-4-performance/hotspots.log). Exact command is recorded in provenance; only this named Playwright spec ran. No production or test source needed modification in this round.
