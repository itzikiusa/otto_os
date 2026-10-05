# Scores and evidence

Baseline source: `03f2bc3e` (PR78). Target: 9.8 in every category and partition/lens. Rubric: [PLAN.md](PLAN.md). These are reviewer judgments over declared coverage, not measured probabilities that the app is defect-free.

| Category | Iteration 4 baseline | Iteration 4 after fixes | Iteration 5 | Iteration 6 |
|---|---|---|---|---|
| Correctness | 7.0 calibrated mean; minimum 6.2 (source review + shared baseline evidence) | Pending | Pending | Pending |
| Performance | Review in progress; prior measurements are not a score | Pending | Pending | Pending |
| Design | 8.1 static mean; minimum 7.0 | Pending | Pending | Pending |
| UX | Review in progress; no prior numerical baseline | Pending | Pending | Pending |

Claude's design baseline: tokens 7.9; layout 7.8; components 7.0; states 8.1; accessibility 9.3; responsive 8.4; copy 8.2; agent/trust 7.7; theming 8.6; information architecture 8.0. Supplied in `/tmp/otto-app-review-coordination-20261005.txt`; underlying reviewer outputs requested for durable evidence. Static only. Earlier 7.1/7.2/7.4 averages lacked a fixed rubric and used different models; do not chart them as comparable progress.

Prior delivery: PR78 merged; 12 CI checks successful; CI 4,612 Rust tests passed (85 skipped), local 1,081 UI tests passed, 98 distinct affected browser cases verified across initial run plus repaired reruns. Full desktop suite was not all green. Headed Chromium/debug-daemon scale and sustained runs completed; native/manual acceptance and long-term leak absence were not established. See `../app-20261004/VERIFICATION.md` and `PERFORMANCE.md` for exact boundaries.

No new score is final until its findings, coverage and execution evidence are recorded. Missing observations cannot silently become full credit.

## Iteration 4 correctness partition baselines

| Partition | Score | Confirmed findings | Evidence |
|---|---:|---|---|
| 1: Sessions/navigation | 7.0 | 1 blocker, 1 minor | Source traces; [report](iteration-4-correctness-1.md) |
| 2: Data/Git/API | 6.4 | 2 blockers, 3 majors | Source traces; [report](iteration-4-correctness-2.md) |
| 3: Content/artifacts | 6.2 | 1 blocker, 2 majors, 1 minor | Source traces; [report](iteration-4-correctness-3.md) |
| 4: Automation/MCP | 6.8 | 1 blocker, 2 majors | Source traces; [report](iteration-4-correctness-4.md) |
| 5: Platform/cloud/assistant | 7.7 | 2 majors, 1 minor | Source traces; [report](iteration-4-correctness-5.md) |

These original reviewer totals are provisional; shared evidence is calibrated below. The five reports identify 5 blockers, 9 majors and 3 minors by source trace, not executed reproduction.

### Shared execution-evidence calibration

Reviewers initially credited shared baseline execution inconsistently. The original reports remain intact. Under the explicit shared execution scale now in PLAN.md, all partitions receive 1.0 for the verified green baseline until named results are mapped to their acceptance matrix. This is calibration of the same evidence, not new testing or a repaired defect.

| Partition | Original coverage /2 | Calibrated coverage /2 | Original total | Calibrated total |
|---|---:|---:|---:|---:|
| 1 | 0.8 | 1.0 | 7.0 | 7.2 |
| 2 | 1.2 | 1.0 | 6.4 | 6.2 |
| 3 | 0.0 | 1.0 | 6.2 | 7.2 |
| 4 | 1.0 | 1.0 | 6.8 | 6.8 |
| 5 | 1.3 | 1.0 | 7.7 | 7.4 |

Calibrated mean = 6.96/10 (reported 7.0), minimum = 6.2. The calibration preserves all source-quality dimension scores and uniformly credits the verified shared baseline; it is not an assertion that all prior tests cover new failures.

Blockers remain open; no category is accepted regardless of arithmetic.
