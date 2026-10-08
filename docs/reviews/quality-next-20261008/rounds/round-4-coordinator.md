# Full round 4 — integration record

Starting commit: `2913a7d17`. Reviewers 16–18 cover correctness, performance, and separate design/UX assessments. This round closes the explicit R3-C3 promotion deduction and independently exercises broader terminal and goal-loop behavior.

The correctness reviewer reproduced four storage-boundary faults before repairing them: failed promotion metadata, a failed forced-promotion waiver, zero affected rows after evaluation removal, and discarded validator-result persistence. Library-write partial success is now an explicit error with an appropriate recovery message; a failed waiver stops before library replacement. Retry completion persists the validator result before rescoring, leaves durable pending signals and an error on failure, and supports retry. The file and database remain separate persistence boundaries; no rollback of a possibly newer user-edited skill or automatic process-restart reconciliation is claimed. [Correctness report](round-4-correctness.md).

The UI reviewer reproduced cross-loop answer/evidence/budget-dialog leakage. Keying the detail component by the selected loop fixes ownership without inventing global draft persistence. Unsaved drafts are discarded on leaving a loop, as on the existing Back path; the regression covers returning to the first loop and a late delete that cannot disturb the newly selected loop's dialog. The coordinator independently inspected light desktop and dark phone captures and reread the keyed ownership repair. [Design/UX report](round-4-design-ux.md).

Coordinator independently reread promotion error ordering, zero-row handling and all set_promoted callers, plus the extracted validator completion boundary. The latter propagates the result-write failure before rescoring and uses conditional terminal persistence, preserving cancellation. No additional confirmed defect was found in this read.

Performance workload, fresh-daemon integration, source hashes and final reassessment are appended after their completion. This file is not a completed-round assertion until that closure is recorded.

## Fresh integration

All **11 named browser cases passed**, zero retries/skips, on rebuilt daemon `14335b30342d5ab8603e9716620d545778a20be7c86775e65dfa1463c00f8dd9`, Node26.10.0, one worker, isolated ports17837/5212. This run includes all final Round4 backend and goal-loop changes. [Execution](../evidence/round-4-coordinator/browser.log), [results](../evidence/round-4-coordinator/browser-results.json), [source/runtime hashes](../evidence/round-4-coordinator/source-manifest.json). UI type/guard check and build passed in the specialist run; subsequent contract changes only add comments to TypeScript. Source whitespace and Rust LOC ratchet passed. Full workspace delivery gates remain with the coordinator after Round5.

## Round closure

The uncontended fresh-daemon WebKit workload passed once: 855/855 rendered markers, 18 flow-control acknowledgements, p95 input-to-render26/34/36ms for1/3/5 terminals, no missed input ticks and zero queues after recovery. All25 sampled owned PIDs and both listeners were gone. [Performance report and raw evidence](round-4-performance.md). These measurements are bounded to the documented hardware and workload, not an external-provider SLA.

All four final scoped assessments are **9.8**: performance, correctness, design and UX. Every confirmed Round4 finding is repaired and verified; partial filesystem/database publication is explicitly reported and recoverable rather than described as atomic. Round4 is complete. Round5 independently challenges the finished branch using the final two new reviewers and a returning independent UI reviewer.
