# Full round 5 — integrated closure

Starting commit `a911ce2bc`. New reviewers 19 and 20 challenged correctness and performance across the cumulative branch; returning reviewer 12 independently assessed design and UX. The campaign has used 20 unique reviewers, with no nested delegation. The first five historical targeted stages count together as round 1; this is the fifth complete round.

Three additional findings were reproduced and repaired:

- A human rating replaced valid custom signal weights with defaults when test and review weights were zero. The real rating-handler regression now preserves the lint-only policy, original command evidence and score headlines. [Correctness report](round-5-correctness.md).
- Retry diff preparation held the publication mutex while awaiting local Git, delaying cancellation and ratings. Diff preparation now precedes that lock; admission still reads fresh status, scoring and validator state while locked. A deterministic paused-diff regression proves cancellation can complete and the old retry cannot restart the cancelled run. Correctness independently reviewed the relocation. [Performance report](round-5-performance.md).
- Expanded Proof evidence could show an old Approval beside a newly saved pending rating. Pending publication now displays neutral pending copy and suppresses old artifacts; recovery fetches current evidence. Both themes reproduced before repair and pass after repair. [Design/UX report](round-5-design-ux.md).

The coordinator reread all three repairs, their failure regressions, the locked retry admission and contract comments. Light desktop and dark phone evidence were inspected directly. No additional confirmed defect remains from that review.

All four specialist assessments are **9.8/10 on the declared reviewed scope**. These are engineering judgments, not test-pass percentages or proof of every product path. Final combined workspace/browser results are recorded in the [campaign report](../FINAL.md). The source hashes there identify the final integrated snapshot; earlier specialist browser execution used the round-4 daemon and is not described as final-backend execution.
