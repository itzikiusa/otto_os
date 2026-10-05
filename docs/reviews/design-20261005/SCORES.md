# Design review scores — 2026-10-05 effort

Rubric (fixed, see REVIEW-BRIEF-AND-RUBRIC.md): start 10.0; −1.0 blocker, −0.3 major, −0.1 minor, −0.03 nit; a systemic pattern counts once. Static code review by 10 lens reviewers (Sonnet); no runtime/screenshots. Re-scores are done by the SAME reviewer agents after the iteration's fixes.

Ledger values are the **fixed-rubric arithmetic** (deductions from each report's own finding counts, two decimals, no rounding). The reviewer's stated headline score is shown separately as **judgement** where it differs; judgement never feeds the average.

| Lens | Iter-4 baseline (main 03f2bc3e) | Iter-4 after fixes | Iter-5 after fixes | Iter-5 reviewer judgement |
|---|---|---|---|---|
| Foundations (tokens/type) | 7.85 | 8.55 | 9.77 | = arithmetic |
| Layout & toolbar | 7.81 | 9.40 | 9.60 | = arithmetic |
| Shared components | 7.01 | 7.84 | 9.37 | ~9.6 |
| States | 8.14 | 9.37 | 9.77 | ~9.8 |
| Accessibility | 9.31 | 9.64 | 9.87 | = arithmetic |
| Responsive & RTL | 8.42 | 9.84 | 9.87 | = arithmetic |
| Copy & content | 8.21 | 8.21 | 8.77 | ~9.2 |
| Agent & trust patterns | 7.71 | 9.01 | 9.90 | = arithmetic |
| Theming & motion | 8.55 | 9.15 | 9.77 | = arithmetic |
| IA & navigation | 7.98 | 8.71 | 9.80 | = arithmetic |
| **Average** | **8.10** | **8.97** | **9.65** | — |

Iteration-4 judgement column (recorded, not averaged): tokens 8.5, states 9.2, theming 9.2 → 8.94. Iteration-5 re-scores: `iteration-5/rescore/` (same reviewers, HEAD 4cc239da).

Earlier passes (PRs #75–#77) were scored without this rubric (≈7.1 / 7.2 / 7.4) and are not comparable.

Iteration-4 after-fix scores are the SAME reviewer agents re-scoring HEAD b751ca6f against their own baseline findings (iteration-4/rescore/). Under the rubric a Partial finding keeps its full deduction, so lenses with large but incomplete sweeps (copy, components) move little until items are fully closed.
