# Quality review continuation — 2026-10-08

Baseline: `7cff9e538deccdd0e346af7c6285e11863285c00` (merged PR #96), clean checkout. Branch: `review/quality-20261008-next`.

User scope: review and fix performance, correctness/bugs, design and UX, aiming for at least 9.8/10 per vertical; at most 20 review agents and five review/fix iterations; one PR. Merge to main only after approval and mandatory checks, then rebuild/reinstall/replace the running app. Approval is pending until the concrete PR exists.

## Execution

- Reviewers run in batches of at most three (session concurrency limit), with no nested delegation. Initial specialists cover performance, correctness, and design/UX. Additional targeted and independent reviewers are assigned as gaps and repairs become concrete, up to 20 total.
- Each iteration consists of evidence-backed review, confirmed repairs, relevant verification and reassessment. Reuse reviewers for repair validation. Stop early only if all four assessments support 9.8; otherwise use up to five iterations and report any remaining shortfall honestly.
- Scores are engineering judgments on explicitly covered surfaces, never test-pass percentages. Whole-product confidence must state native, external-provider, accessibility and workload limits separately. Prior 9.2–9.3 scores are historical context, not current results.
- Findings require precise code locations, a reachable trigger and observable consequence. Performance findings require input size/cost. UI findings require visible or interaction evidence where practicable. Do not manufacture changes to increase a score.
- One Cargo lease and one heavy browser/native lease. Only named local Playwright specs; isolated throwaway daemon/profile, unique ports, no orphan sweep, no real user data changes. Node 26.10.0 and Rust stable 1.99.0.
- Preserve other work, inspect staging before commits, no force push/history rewrite. Contracts/types change with API behavior. Use repository regression and integration gates; do not weaken thresholds or convert failures to skips.

## Round accounting correction

The initial delivery incorrectly called five targeted stages five complete iterations. Those stages collectively count as **round 1**. A complete round must review all four verticals, repair confirmed findings, verify the changes, and reassess every vertical before the next round starts. Historical report filenames are retained as provenance, not round counts.

The campaign will use **20 reviewers total**, reusing reviewers for repair checks. Reviewers 1–9 completed round 1; 10–12 cover round 2. Full rounds 3–5 follow sequentially. Every score deduction requires a concrete finding or bounded validation task with an acceptance condition. Product quality and evidence confidence are reported separately. Scores will not be raised to satisfy the target without substantiation.

## Historical stages within round 1

Stage 1: initial review and repairs:

1. Performance: sessions/PTY/transcript/search/browser/proxy resource bounds and sustained-load gaps.
2. Correctness: workflow/evaluator/publication/cancellation/persistence and ownership boundaries.
3. Design/UX: shared shell, navigation, focus, drafts, failure recovery and representative rendering.
4. Coordinator: baseline/provenance, integration gates, native/deployment constraints and additional product seams.

Stage 2: independent browser/UI and native composition review. Found and repaired cached-request burst rejection; retained native accessibility and modal-resume uncertainty.

Stage 3: evaluator independent review plus uncontended 1/3/5-terminal runtime measurements. Repaired human-rating publication, incomplete-pass acceptance and winner selection.

Stage 4: independent populated design/UX journeys and native-evidence audit.

Stage 5: final independent changed-code review and integration gates. Found legacy retry badges omitted failed validators; repair and persisted regression passed. Strengthened native visible/unfocused assertion and passed the fresh-daemon native probe. Nine review agents used, no nested delegation. These were stages within round 1; four further complete rounds are now required before that five-round claim is valid.

## Delivery gates

Create a single PR after repairs and verification. The PR must state actual scores and unresolved evidence limits. Once approved, gate merge only on mandatory checks (advisory failures remain reported). Rebuild and deploy from the clean merged commit with the existing receipt/signature/rollback workflow in `packaging/deploy.sh`, then verify the installed and running app/daemon match the receipt.
