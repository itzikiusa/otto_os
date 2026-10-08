# Performance iteration 1 — sessions, transcripts, search and browser

**Verdict: approve the repaired browser resource paths with verification limits.** Three major resource findings repaired; zero remaining confirmed findings in this inspected slice. Baseline `7cff9e538`, branch `review/quality-20261008-next`. This does not certify every session/PTY/search path or the whole app.

**Assessment:** inspected browser admission/lifecycle performance **9.2/10**, medium-high confidence in deterministic bounds; overall assigned performance slice **8.8/10**, medium confidence. These are engineering judgments, not measured quality percentages. A 9.8 score is unsupported by absent real-Chromium flood/soak, allocator/RSS, native and concurrent cross-module measurements.

## Confirmed findings and repairs

### Major P-01 — CDP event, output and reply-waiter retention lacked admission

**Locations:** `crates/otto-browser/src/live/conn.rs:32`, `:204`, `:251`, `:341`; route construction in `session.rs:258` and receiver type propagation in `chrome.rs:91`.

**What:** the original transport had unbounded output, per-page event and browser-event channels. The existing 96 MiB individual frame ceiling did not bound retained aggregate frames. Independently, a browser that drained command bytes but did not answer let the pending-call map grow until the 30-second call timeout. HTTP callers and navigation-history refreshes reach this path.

**Cost:** with event production rate R, consumption C, average event payload B and duration T, backlog grows by approximately max(0,R-C)*B*T, plus JSON/container overhead. For 1,000 surplus events/second at 4 KiB each, wire payload alone grows about 234 MiB/minute. Reply waiters grow to request-rate*30 seconds; the 1,024-call real-pipe probe reproduced retention above 256 even while the peer drained output. These are models/probes, not observed production traffic rates.

**Evidence:** the unchanged implementation failed actual duplex-pipe output-flood and event-flood regressions; event overload also failed to settle an existing pending call. A separately added draining/no-reply regression then failed against the first queue repair. No live daemon or private data was involved.

**Fix/payoff:** bounded channels (256 messages), independent 32 MiB output and conservative retained-event charge budgets, and 256 pending reply slots. The event budget is shared across browser/page routes and stays held during guard work. `CdpEvent` no longer implements Clone, avoiding an uncharged deep-clone surface. Queue overload closes the transport and fails pending commands; it never silently loses a critical Fetch event. Waiter-only overload returns a capacity error. Shutdown now signals `closed()` and settles waiters on all explicit/error paths. This changes unbounded duration-dependent retention to fixed admission bounds. Accounting is not a promise of exact RSS; one current inbound frame can still reach 96 MiB, and command construction happens before queue admission.

**Tests:** `output_flood_closes_connection_instead_of_retaining_commands`, `event_flood_closes_connection_and_fails_pending_calls`, `unanswered_calls_have_bounded_admission_even_when_output_drains`, separate event/output byte-budget probes, and `sustained_event_drain_releases_budget_and_preserves_reply_routing`. Existing normal reply routing, cancellation and protocol-error controls remain green.

### Major P-02 — the guard semaphore bounded DNS but not spawned work

**Locations:** `crates/otto-browser/src/live/process.rs:321`, `:350`; `session.rs` Fetch.requestPaused handler.

**What:** both browser-level and per-page handlers spawned a task per paused request before awaiting one of 64 guard permits. A remote page can generate many subresource fetches; slow DNS left those tasks and event payloads queued outside the semaphore. Browser/process/session count limits did not bound requests per page.

**Cost:** N paused requests created N tasks and retained O(sum(payload bytes)) even when only 64 could vet DNS. For 10,000 paused events with 4 KiB request data, that is at least 39 MiB of wire payload plus parsed values/task state; large headers/bodies increase it. The page and DNS timing determine N, so these are plausible workload illustrations, not production measurements.

**Evidence:** a production ChromeProcess fixture with all guard slots occupied received a real CDP paused event. The original code failed to send a rejection within one second; after repair it sends Fetch.failRequest with the correct request ID. This verifies the actual event handler and pipe command, not only semaphore mechanics.

**Fix/payoff:** acquire a permit synchronously before spawning; reject overload with BlockedByClient. Retain the event byte lease through the admitted task, time-limit vetting to 15 seconds, and cancel admitted work on transport closure. At most 64 admitted workers per process; no unbounded semaphore waiter backlog. Both event ingress paths use the same method. Approval/screenshot work retains a slot intentionally, so slow outward approvals can consume capacity; safe overload rejection is the documented tradeoff.

### Major P-03 — proxy accepted sockets had neither a cap nor owner cleanup

**Locations:** `crates/otto-browser/src/live/proxy.rs:49`, `:92`, `:98`.

**What:** every accepted loopback connection spawned an untracked task. Dropping GuardProxy aborted only acceptance, leaving accepted handshakes/tunnels alive. Browser connections, preconnects and local clients reach this listener; established tunnels could persist indefinitely.

**Cost:** N connections retain N tasks plus N client descriptors, and another N upstream descriptors/transfer buffers for connected tunnels. One thousand established tunnels imply roughly 2,000 socket descriptors. Even incomplete handshakes accumulate arrival-rate*10 seconds sockets before the old timeout. No macOS/browser-specific maximum was assumed.

**Evidence:** after real SOCKS method negotiation, dropping the proxy left the accepted client open beyond one second on the original code. The repaired test sees closure. Repeated saturation opens 128 real clients, verifies immediate closure of the excess socket, drops clients and repeats eight times, proving slots are reclaimed.

**Fix/payoff:** the accept task owns a JoinSet capped at 128; excess sockets are closed before a worker is created, completed tasks are reaped, and dropping the owner aborts all accepted tasks. DNS vetting also gets a 15-second deadline. O(N) unbounded accepted work becomes at most 128 active socket workers. Persistent healthy tunnels remain supported. The automated drop fixture exercises handshakes, not a real external established tunnel; task ownership covers both at source level.

## Executed verification

Durable logs are in `evidence/performance/`; source SHA-256 checkpoint is `source-checkpoint.json` there. Cargo commands were coordinated with the other reviewer; no simultaneous Cargo run was intentionally started.

- `cargo test -p otto-browser --lib live:: -- --nocapture`: baseline reproduction 65 passed / 3 expected failures (`red.log`).
- Same command after queue/proxy repair and adding guard reproduction: 68 passed / 1 expected guard failure (`guard-red.log`).
- `cargo test -p otto-browser --lib unanswered_calls_have_bounded_admission_even_when_output_drains -- --nocapture`: 1 expected failure (`pending-red.log`).
- Final live group: 74 passed / 0 failed (`green2.log`).
- `cargo test -p otto-browser`: **125 unit tests + 3 integration tests passed**, zero failures; zero doc-tests (`full.log`). The integration tests use their existing controlled CDP peers; this is not real Chromium fidelity evidence.
- `cargo clippy -p otto-browser --all-targets -- -D warnings`: passed (`clippy.log`). Owned Rust files formatted; `git diff --check` passed.

Repeated bounded-load sample: 10,000 actual duplex-pipe CDP events with 4 KiB text payloads (~40 MiB source) drained in **138.65 ms**, followed by a correctly routed command reply. Eight saturation/recovery rounds accepted **1,024 real loopback sockets** in **97.84 ms**, rejecting each excess connection. Byte-budget saturation is separately exercised. These are one-run local observations, not p95 latency, sustained wall-clock soak, CPU/RSS ceilings or a hardware/native benchmark. The positive drain path proves budget reuse beyond a total input volume exceeding the budget.

## Coverage and limits

Read targeted limitations and prior evidence in R02/R07/R13/R15; ran the skill hotspot script against the pinned base (no source diff existed initially). Deep current-source coverage: live CDP transport/dispatch/call cancellation, session event routing/frame flow, guard worker dispatch, guard verdict cache, browser process setup/lifecycle and shared SOCKS proxy. Positive bounds include viewer window=2 plus one newest pending frame, viewer output queue=64, guarded DNS cache=1024, max Chromium processes=4.

Sampled current transcript tail/cache/read paths: 64 live tails, shared two-worker fold admission, eight pending folds, per-key waiter limit eight, live retained charge 128 MiB, offline cache 128 MiB, interactive streamed input ceiling128 MiB and fold charge256 MiB. Tail generation identity and fallback refresh paths were read; no newly confirmed issue reported. These are source accounting limits, not measured combined RSS.

Sampled unified-search state projections and route permission fanout. Eight narrow metadata sources replace prior full-record/N+1 loading, retaining at most five results per source. No-match substring matching remains O(N) and can require SQL sorts; no million-row or multi-request contention run was performed. No unsupported indexed-search claim is made.

Sampled PTY holder input/ACK/output lifecycle and writer chunking; did not exhaustively audit holder transports, all provider adapters, session manager, transcript folding corpus, UI rendering, or browser install/login/native integration. The unbounded-looking holder queues need caller/serialization sizing before treating them as findings; they are not asserted defects in this report.

No real Chromium flood, external-site load, long wall-clock soak, process allocator/RSS measurement, established-proxy-tunnel teardown probe, native WKWebView stress, or cross-module contention workload ran. Server consumer compilation and root full-workspace gates are coordinator-owned and pending. API/feature documentation records limits and overload behavior; no DTO shape or migration changed. Only the authorized five browser files, two documentation paragraphs and this evidence/report were edited by this reviewer; other agents' changes were preserved.
