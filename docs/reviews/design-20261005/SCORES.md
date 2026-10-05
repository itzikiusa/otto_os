# Design review scores — 2026-10-05 effort

Rubric (fixed, see REVIEW-BRIEF-AND-RUBRIC.md): start 10.0; −1.0 blocker, −0.3 major, −0.1 minor, −0.03 nit; a systemic pattern counts once. Static code review by 10 lens reviewers (Sonnet); no runtime/screenshots. Re-scores are done by the SAME reviewer agents after the iteration's fixes.

Ledger values are the **fixed-rubric arithmetic** (deductions from each report's own finding counts, two decimals, no rounding). The reviewer's stated headline score is shown separately as **judgement** where it differs; judgement never feeds the average.

| Lens | Iter-4 baseline (main 03f2bc3e) | Iter-4 after fixes (arithmetic) | Reviewer judgement (after) |
|---|---|---|---|
| Foundations (tokens/type) | 7.85 (3M/11m/5n) | 8.55 (2M/7m/5n) | 8.5 (rounded down) |
| Layout & toolbar | 7.81 (3/12/3) | 9.40 (0/6/0) | 9.4 |
| Shared components | 7.01 (7/8/3) | 7.84 (5/6/2) | 7.8 |
| States | 8.14 (3/9/2) | 9.37 (0/6/1) | 9.2 (downgraded: loader pattern still widespread) |
| Accessibility | 9.31 (1/3/3) | 9.64 (0/3/2) | 9.6 |
| Responsive & RTL | 8.42 (3/5/6) | 9.84 (0/1/2) | 9.8 |
| Copy & content | 8.21 (2/11/3) | 8.21 (2/11/3) | 8.2 |
| Agent & trust patterns | 7.71 (2/16/3) | 9.01 (1/6/3) | 9.0 |
| Theming & motion | 8.55 (1/10/5) | 9.15 (0/7/5) | 9.2 (report's headline counted 4 nits; its table lists 5) |
| IA & navigation | 7.98 (2/13/4) | 8.71 (2/6/3) | 8.7 |
| **Average** | **8.10** | **8.97** | 8.94 |

Earlier passes (PRs #75–#77) were scored without this rubric (≈7.1 / 7.2 / 7.4) and are not comparable.

Iteration-4 after-fix scores are the SAME reviewer agents re-scoring HEAD b751ca6f against their own baseline findings (iteration-4/rescore/). Under the rubric a Partial finding keeps its full deduction, so lenses with large but incomplete sweeps (copy, components) move little until items are fully closed.
