# Design review scores — 2026-10-05 effort

Rubric (fixed, see REVIEW-BRIEF-AND-RUBRIC.md): start 10.0; −1.0 blocker, −0.3 major, −0.1 minor, −0.03 nit; a systemic pattern counts once. Static code review by 10 lens reviewers (Sonnet); no runtime/screenshots. Re-scores are done by the SAME reviewer agents after the iteration's fixes.

| Lens | Iter-4 baseline (main 03f2bc3e) | Iter-4 after fixes |
|---|---|---|
| Foundations (tokens/type) | 7.9 (3M/11m/5n) | 8.5 |
| Layout & toolbar | 7.8 (3/12/3) | 9.4 |
| Shared components | 7.0 (7/8/3) | 7.8 |
| States | 8.1 (3/9/2) | 9.4 (reviewer rounded to 9.2) |
| Accessibility | 9.3 (1/3/3) | 9.6 |
| Responsive & RTL | 8.4 (3/5/6) | 9.8 |
| Copy & content | 8.2 (2/11/3) | 8.2 |
| Agent & trust patterns | 7.7 (2/16/3) | 9.0 |
| Theming & motion | 8.6 (1/10/5) | 9.2 |
| IA & navigation | 8.0 (2/13/4) | 8.7 |
| **Average** | **8.10** | **8.96** |

Earlier passes (PRs #75–#77) were scored without this rubric (≈7.1 / 7.2 / 7.4) and are not comparable.

Iteration-4 after-fix scores are the SAME reviewer agents re-scoring HEAD b751ca6f against their own baseline findings (iteration-4/rescore/). Under the rubric a Partial finding keeps its full deduction, so lenses with large but incomplete sweeps (copy, components) move little until items are fully closed.
