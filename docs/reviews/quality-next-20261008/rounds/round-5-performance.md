# Full round 5 — independent performance review

**Verdict: Approve. Open findings: 0 blockers, 0 major, 0 minor, 0 nits. Performance: 9.8/10 for the inspected browser admission, terminal/transcript retention and evaluator publication scope.** One minor lock-lifetime issue was reproduced, repaired and verified in this round. Confidence is high for the traced bounds/concurrency regression and medium-high for the audited runtime configurations; the score is an engineering acceptance assessment, not a measured whole-product percentage.

Reviewer **20/20**, starting commit `a911ce2bc614ae430bf9a16bcd9ee7614813d9d4`, cumulative source diff against `origin/main`. This pass applies the performance-review skill and independently checks both source cost and the retained execution evidence.

## Repaired finding

### [minor] Retry preparation delays cancellation behind Git — `crates/otto-server/src/skill_eval.rs:3121`

**What:** Validation retry acquired the evaluation's publication mutex before preparing its Git diff. Cancellation, rating publication and promotion require the same mutex. Diff gathering does not mutate the evaluation, yet a slow repository kept these operations waiting.

**Cost:** Let `Tgit` be the retry's local diff-preparation time and `k` the number of concurrent actions on the same evaluation. The old lock imposed up to `Tgit` additional waiting on each of those `k` actions. Each local Git command has a default 30-second budget (`crates/otto-git/src/local.rs:131`); fork-point discovery plus diff preparation can invoke more than one command. This is an interactive retry path on a large or slow worktree, not a claim that normal small-repository diffs take 30 seconds. The issue is bounded latency coupling, not unbounded global locking or a reproduced deadlock.

**Evidence:** The real retry handler was paused at its diff boundary using a test-only hook keyed to the fixture's temporary worktree. While that pause remained held, the real cancellation path failed to complete within one second. [Red regression](../evidence/round-5-performance/retry-lock-red.log). The pause proves the lock dependency deterministically; it does not measure disk speed.

**Fix/payoff:** Prepare the capped diff before taking the publication lock. Under the lock, keep the fresh evaluation read and status/iteration/validator/scoring selection before atomic retry admission. The initial snapshot only selects the worktree; a concurrent human rating remains authoritative and cancellation invalidates admission. Lock-dependent cancellation latency no longer includes `Tgit`; the number of database reads is unchanged. The regression also verifies that resuming the old retry cannot change the cancelled run to validating, leaves the existing validator status intact, and releases its retry lease. Reviewer 19 independently inspected this ordering and found no conflicting correctness issue.

The pause, keyed registry and test module are `cfg(test)` only. No real provider, user repository, running daemon or app participates.

## Sized source review

- **Browser CDP:** 256 unanswered calls, 256 queued messages per event route, separate 32 MiB output and shared 32 MiB retained-event accounting; one inbound wire message is independently capped at 96 MiB. Event byte permits stay alive through guard work. Normal dispatch moves the JSON params rather than cloning the complete event. Saturation deliberately closes the connection; these are retained-queue accounting limits, not exact process RSS or a claim that a JSON value never allocates transient parsing memory.
- **Browser guard/proxy:** 64 permits are acquired before guard-task spawning. Fresh positive safe-method origin hits continue inline; negative/expired/unknown origins and outward methods retain guarded handling. The SOCKS layer independently vets each dial, limits accepted tasks to 128, streams bodies, and owns those tasks through a JoinSet. Increasing offered load 10× or 100× reaches admission bounds rather than retaining every arrival. It does not promise lossless unlimited load.
- **Failure hints:** At most 1,024 hosts survive in fallback history. Existing-host update and successful-path lookup are O(1); a newly failing origin performs a bounded expiry/oldest-entry scan. There is no justification for replacing this small failure-path scan with a more complex cache solely to improve asymptotic notation.
- **Evaluator publication:** The new mutex registry uses weak entries and prunes expired entries on acquisition; its synchronous map lock is released before awaiting the per-evaluation mutex. Registry scans are O(number of concurrently retained evaluation locks), not lifetime evaluations. Commands and agents execute outside the publication mutex. Proof recomputation takes its own per-pack lock, with no reverse acquisition of an evaluation lock found in the traced call graph. Winner reselection is one indexed query over at most ten admitted iterations, followed by at most ten score decodes; `idx_skill_eval_iterations_eval(eval_id, iter)` ships in migration 0017 and admission rejects more than ten iterations or sixteen expanded validators. This is a bounded warm action, not an N+1 or a growing table scan. Same-evaluation serialization still intentionally covers durable publication and promotion.
- **PTY and transcript sample:** The actual websocket producer reserves input bytes before using its unbounded-channel type; ordinary input has a 1 MiB queue plus 1 MiB deferred budget, with a single oversized frame admitted alone. Held-PTY writes wait for their acknowledgement. Output uses credit and bounded snapshot recovery; xterm writes at most two 64 KiB slices concurrently. Offline transcript folds have two workers, eight pending identities/eight waiters per identity, and 32 entries/128 MiB retention; live tails cap at 64 and reserve per-tail/global charge before folding. These limits concern the inspected producer/cache paths, not all intentionally requested historical transcript pages.

The hotspot script seeded this sweep; its matches are [retained candidates](../evidence/round-5-performance/hotspots.log), not asserted findings. No other confirmed performance defect survived input sizing and surrounding-code checks.

## Independent evidence audit

I checked the assertions and test seams rather than treating earlier reviewers' scores as evidence. [Audit result](../evidence/round-5-performance/evidence-audit.json).

- All **nine** recorded round-3 source/executable/browser hashes still match. The positive HTTP test uses an exact test-only socket substitution after unchanged production vetting, refuses other destinations, checks each of 384 distinct uncached response bodies, requires actual text input and a fresh ACKed frame per round, and closes its profile/listeners. Its retained execution shows 384 bodies, zero network failures, 23 frames and successful teardown. The fixture does not measure real Internet DNS, TLS or HTTP/2.
- Of round 4's 23 recorded inputs, **22 matched** at audit time, including the measured daemon, terminal/transcript sources, fixture and dependency locks. Only evaluator source changed for this round's fixes/test hooks; these do not alter the terminal path. The round-4 daemon is therefore valid historical terminal evidence, not falsely described as rebuilt with round-5 evaluator fixes.
- Independently recomputed from raw round-4 observations: **855/855 rendered markers**, nearest-rank p95 **26/34/36 ms** for 1/3/5 terminals, **18 actual ACKs**, zero fatal UI errors, and all final terminal queues empty. The test observes xterm render callbacks after real clipboard → websocket → owned cat PTY delivery; it does not merely time a mocked response. Its gate stayed at one second. Cleanup records all 25 sampled owned PIDs gone, the fixture data directory removed, and dedicated listeners absent.
- That workload is approximately one KiB/second per terminal with 90 load samples and ten recovery samples. It establishes responsive forward progress and quiet queue drainage in that configuration. It does not establish high-throughput terminal saturation, native Tauri performance, provider inference speed, a long-duration memory-leak guarantee, or combined cross-module contention. Summed RSS can double-count shared pages. These remain explicit confidence boundaries, not invented product defects or undefined score penalties.

No unchanged long benchmark was repeated: exact-input continuity and the newly reproduced evaluator wait warranted a targeted concurrency regression instead.

## Final verification and assessment

- Initial exact regression: **failed as expected**, because cancellation waited on the held diff. [Red output](../evidence/round-5-performance/retry-lock-red.log).
- After repair, `cargo test -p otto-server --lib skill_eval:: -- --nocapture`: **33 passed, zero failed/ignored**, including the new cancellation/admission regression and the current independent correctness repair. [Green output](../evidence/round-5-performance/retry-lock-green.log).
- `cargo clippy -p otto-server --all-targets -- -D warnings`: **passed**, 14.00 seconds. [Output](../evidence/round-5-performance/clippy.log).
- Owned source formatting and source/report whitespace checks passed. [Final source hashes and commands](../evidence/round-5-performance/provenance.json). The coordinator owns final workspace/consumer integration gates.

The concrete deduction identified in this round is closed. No unresolved product deficiency is hidden behind the remaining generalization limits; there is no undefined additional benchmark or arbitrary work required to meet this scope's 9.8 acceptance bar.
