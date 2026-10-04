# Iteration 1 — performance partition 1

**Verdict: Approve with fixes.** Counts: blocker 0 · major 4 · minor 1 · nit 0 · question 0.

Baseline: `a16f4c71`. Read-only review of the existing shell/navigation, agents/session UI, terminal transport/PTY ring, conversation store, transcript routes/live tails, and relevant session lifecycle paths. Applied the performance-review skill with desktop workloads rather than its casino examples. The skill's hotspot script reported no changes against this baseline; findings below come from tracing existing code.

**Scope sized:** 1–10 concurrently busy/viewed agents, up to 64 admitted live transcript tails, transcripts with thousands of turns and 20–100 MiB of retained folded payload, and newline-free terminal streams after the 2 MiB ring fills. These are workload assumptions, not measurements of the user's machine. No builds, tests, servers, or profiling were run by this reviewer. CPU/RAM validation belongs to the parent review's measurement pass.

## Ranked findings

### P1-1 [major] Newline-free output copies the entire 2 MiB ring for every 8 KiB input — `crates/otto-pty/src/ring.rs:88`

**What:** The single-line overflow fix bounds memory, but `Vec::split_off(excess)` allocates and copies the retained suffix each time a chunk exceeds the cap. The next append can also grow the freshly allocated vector. This runs synchronously inside the ring mutex in `PtyMirror::feed_bytes` (`crates/otto-pty/src/lib.rs:358`).

**Cost:** Let B = retained bytes (default 2 MiB) and c = incoming chunk (local PTY reads at most 8,192 bytes, `lib.rs:65,509`). After a newline-free stream fills B, each c-byte output event copies approximately B bytes: O(B/c) amplification, at least 256 retained bytes copied per input byte at 8 KiB chunks. A 1 MiB/s newline-free producer implies roughly 256 MiB/s of suffix copying per terminal, excluding reallocation; multiple busy terminals multiply this. ANSI cursor redraws, carriage-return progress output, and printing large minified/JSON data are reachable inputs. Smaller read chunks worsen amplification.

**Evidence:** Confirmed by the push → evict → split_off path. The ring's line and byte bounds are present; this is a copy-cost finding, not an unbounded-memory claim. No wall-clock throughput measured.

**Fix:** Store a logical head offset/chunk deque for an incomplete line, or use a byte ring and line boundaries, so dropping a prefix advances a cursor and copies are amortized. Preserve the newest B bytes and current search/tail semantics. Per incoming event changes from O(B + c) to amortized O(c); validate with newline-free and carriage-return streams as well as ordinary newline floods.

### P1-2 [major] Live transcript folds have a count cap but no byte budget — `crates/otto-server/src/transcript_tail.rs:264`

**What:** Each live tail initially reads the entire JSONL into a vector, folds all history, and retains that full `Folder` while the view keeps touching it. The folder's turns, tool-call index, and artifacts grow with subsequent records (`crates/otto-transcript/src/fold.rs:176,186,194,274`). Admission checks only `MAX_TAILS = 64` (`transcript_tail.rs:35,133`); it does not use `TranscriptCache`'s byte budget or its two-worker semaphore.

**Cost:** n = total retained transcript payload per open live session, A = admitted tails. Steady retained payload is O(A × n), and startup temporarily also holds each raw file. With A=10 and a folded payload of 20 MiB/session, the base folds alone retain about 200 MiB; at 100 MiB/session that is about 1 GiB, before indices, temporary JSON, snapshots, PTYs, and provider processes. Cost grows with session history even when the UI displays only the last 60 turns. Concurrent opens can start A independent blocking folds. These figures describe payload arithmetic, not measured RSS.

**Evidence:** Confirmed storage and admission paths. Explicitly checked mitigating bounds: tails stop two minutes after their last touch or 60 seconds after process exit; snapshots expire after ten seconds; offline fold cache is bounded. None bounds the size of a continually viewed live fold. Tool-result text caps bound individual fields, not turn/tool count.

**Fix:** Add aggregate retained-byte admission/accounting and a shared limit for initial folds. Longer term, keep a bounded live tail plus lightweight indexing/checkpoints for older pages, preserving tool-result links and cumulative stats. At minimum, oversized tails should fall back to a bounded/indexed read strategy. Payoff: memory bounded by a configured budget rather than A × full lifetime history, with bounded simultaneous parse peaks.

### P1-3 [major] Fetching one page clones the entire live history — `crates/otto-server/src/transcript_tail.rs:235`

**What:** `page_of` obtains a full `folder.snapshot()` before the route selects its requested page. `Fold::snapshot` is `self.clone().finish(...)` (`crates/otto-transcript/src/fold.rs:549`), cloning turns, tool indices, and artifacts. Every append invalidates this snapshot (`transcript_tail.rs:416`), so page reads against a working session repeatedly pay the full-history copy.

**Cost:** n = full folded payload; p = returned page payload (normally ≤2 MiB, with the single-turn exception). Each page read after a changed fold costs O(n) allocation/copy before O(p) response work, under the per-tail mutex. For a 50 MiB fold serving a 1 MiB page, roughly 50 MiB is copied to return that page; ten separately active chats have ten such copies on reconnect/read, and oversized-delta recovery can repeat it during live output. The ten-second snapshot TTL limits idle retention but not copy size or invalidation frequency.

**Evidence:** Confirmed call chain: `get_transcript` → `live_page_settled` → `page_of` → snapshot → route `page`. This is distinct from P1-2's steady retention: even a fold admitted under a byte budget still incurs these repeated full copies.

**Fix:** Add a page-producing method on the live folder which selects the relevant turn range and clones only selected turns, attaching pending notes/subagent metadata as required. Alternatively store immutable shared turns and snapshot references rather than deep cloning bodies. Keep fold work off runtime workers. Payload copying changes from O(n) to O(p) (plus bounded metadata), and the tail lock is held for less time.

### P1-4 [major] One “Load earlier” permanently disables live history trimming — `ui/src/lib/stores/transcript.svelte.ts:253`

**What:** A successful earlier-page read sets `holdCap = true`. Subsequent deltas explicitly skip `trimTurnHead` at line 299. There is no reset when the reader returns to the live tail: the flag resets only when the transcript is fully replaced (line 218). The inactive 24-conversation/32 MiB eviction runs only on view release (lines 492–509), so it cannot bound a chat left open.

**Cost:** n = turns accumulated after that action, d = turns in an incoming delta. Retained payload grows O(n); each delta copies the n-entry array and does up to d linear searches (`lines 280–283`, O(n × d)). A long-running chat accumulating 5,000 turns averaging 8 KiB retains about 39 MiB of turn text alone rather than the ordinary 500–600-turn window; larger tool-heavy turns increase this. Rendering is capped at 300 turns, but that cap does not release the store's history. New records continue to accumulate while the user remains on that chat.

**Evidence:** Confirmed all `holdCap` reads/writes and inactive eviction lifecycle. The normal no-history-pagination live path is bounded and is not flagged.

**Fix:** Preserve the currently viewed historical window separately from a bounded live-tail window; unload remote pages behind explicit cursors. A smaller first change is to clear the hold and apply trimming when returning to live-follow mode, while retaining the historical anchor during active reading. Apply a byte budget as well as a turn count. Payoff: O(window payload) retained memory and O(window × d) delta work independent of session uptime, without deleting historical data on disk.

### P1-5 [minor] Transcript fallback resolution walks provider directories on a Tokio worker — `crates/otto-server/src/routes/transcript.rs:188`

**What:** The async resolver directly calls `resolve_transcript_sync`. If the saved path is absent/stale, Codex resolution calls the recursive, sorted synchronous directory walk in `crates/otto-sessions/src/manager.rs:318`; Claude's fallback performs a metadata check per project directory (`crates/otto-sessions/src/lifecycle.rs:145`). The corresponding session-capture path already offloads the same resolution (`manager.rs:371`).

**Cost:** n = entries visited under the provider history root. A missing/old Codex id may inspect the full root (O(n) filesystem operations, plus sorting per directory) in a single non-yielding runtime task. Tens of thousands of history entries, a cold filesystem cache, or concurrent initial/missing-transcript opens can delay unrelated socket/HTTP tasks scheduled to those workers. A successful saved-path lookup is O(1) and the successful fallback is persisted, so this is narrower than the major hot paths above; unresolved paths can repeat.

**Evidence:** Confirmed direct synchronous call and fallback implementation. No latency measurement; ranked minor because normal saved-path reads avoid the tree walk.

**Fix:** Clone owned resolver inputs into the existing `offload::blocking` helper, then persist the returned path asynchronously. Consider a short, invalidated negative lookup cache for repeatedly unresolved ids. Directory work remains O(n), but runtime-worker blocking changes to an async wait, protecting terminal responsiveness during history misses.

## Bounds verified / no new finding

- Terminal transport uses credit flow control, bounded held output, snapshot recovery, and off-loop reauthorization (`crates/otto-sessions/src/ws.rs`); raw PTY ring bytes/lines are bounded. The newline-free copy amplification above remains despite that memory bound.
- Ordinary conversation rendering has a 300-turn mounted window and an initial 12-turn mount; unchanged turns retain identity. Inactive conversation eviction is capped at 24 entries/32 MiB, and read concurrency is two (`transcriptLifecycle.ts`). These are effective protections in the ordinary path.
- The offline transcript fold cache limits entries, bytes, pending distinct folds, and concurrent folds (`transcript_cache.rs`). Those protections do not wrap live-tail admission.
- Session status writes occur on transitions, status UI updates are coalesced, and foreground/archived filtering keeps hidden session history out of the ordinary sidebar. Shell module pages load lazily and share in-flight load promises.

## Measurement handoff

Use `ui/scripts/loadtest/loadtest.mjs` and `analyze.mjs` as the primary harness; its README describes isolated data/HOME, fake providers, safety aborts, and CPU/RSS + page/WS/API metrics. Run one workload at a time. Start with 1 and 3 agents, then 5 if machine headroom allows, comparing quiet-agent baseline, busy focused terminal, tiles, and Home. Run a 15-minute leak phase with 3–5 agents and record slopes/recovery after closing views; do not infer stability from a short scale run. Parent must also sample the real application separately without changing user sessions.

Focused workloads for these findings:

1. Compare 20 MiB terminal output with ordinary newlines versus no newlines/carriage-return redraws. Sample daemon CPU and typing/probe delay after the ring reaches 2 MiB. Existing `ui/e2e/desktop-terminal-flood-perf.spec.ts` uses newline-heavy frames and mocks the daemon, so it does not establish P1-1's daemon cost.
2. Open 1/3/5 synthetic large live transcripts, measuring raw file size, folded payload estimate, peak/steady daemon RSS, and recovery after the two-minute tail lease. Include repeated page requests while appending records to expose P1-3 independently of startup folding.
3. Use the conversation fixture to click “Load earlier” once, return to the tail, append thousands of small turns, and compare client heap/store count against an identical chat without that click. Existing mounted-DOM gates alone cannot catch P1-4.
4. Create many disposable Codex dated history entries and request a missing id while probing an unrelated terminal/health route. Compare runtime lag before/after offloading resolution.

Useful existing focused suites: `desktop-conversation-load-perf.spec.ts` (cold/reopen/reload/append-while-loading; optional copied fixture), `desktop-conversation-perf.spec.ts`, `desktop-agents-scale-perf.spec.ts` (2,000 background/300 archived mocked sessions), `desktop-shell-perf.spec.ts`, `desktop-nav-smooth-perf.spec.ts`, and `crates/otto-server/tests/runtime_lag.rs`.

## Coverage limits

This is a focused static/hand-traced pass, not a claim that the full application meets a performance budget. No native WebKit or Rust allocator measurements were taken, and every numeric cost above is arithmetic from code/workload assumptions. Tauri native navigation/window internals, every provider adapter's folding semantics, and every manager maintenance sweep were not exhaustively audited. No source changes or external writes were made; only this review file was added.
