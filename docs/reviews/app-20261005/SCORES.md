# Scores and evidence

## Final bounded result — iterations 4–6 complete

| Category | Final mean | Minimum partition/lens | Evidence scope |
|---|---:|---:|---|
| Correctness | **9.88** | **9.8** | Repaired matrix independently rechecked; named regression evidence |
| Performance | **9.20** | **8.9** | Current scale/measured UI work; full sustained run and RSS attribution incomplete |
| External static design | **9.96** | **9.80** | External ten-lens assessment; rendered/native limits remain |
| Internal static design | **10.00** | **10.00** | Five bounded repair partitions; distinct from the external ten lenses |
| UX | **9.82** | **9.8** | Named task/recovery journeys; remaining native/mounted limits explicit |

**The overall every-partition 9.8 target was not reached: performance remains below it.** No confirmed blocker/major remains open in the bounded repair matrix. These are review judgments, not whole-app reliability percentages. Final dimensions, fixed-rubric deductions and unresolved acceptance work: [iteration-6-scores.md](iteration-6-scores.md). Iteration 5 preserved checkpoints: [iteration-5-scores.md](iteration-5-scores.md). Execution: [VERIFICATION.md](VERIFICATION.md); measurements: [PERFORMANCE.md](PERFORMANCE.md).

## Historical checkpoints — preserved

The table and narrative below retain their original provisional status and numbers. Their “Pending” entries describe that earlier checkpoint and are superseded by the final table above; they are not current delivery status.

Baseline source: `03f2bc3e` (PR78). Target: 9.8 in every category and partition/lens. Rubric: [PLAN.md](PLAN.md). These are reviewer judgments over declared coverage, not measured probabilities that the app is defect-free.

| Category | Iteration 4 baseline | Iteration 4 after fixes | Iteration 5 | Iteration 6 |
|---|---|---|---|---|
| Correctness | 7.0 calibrated mean; minimum 6.2 (source review + shared baseline evidence) | **Provisional 8.72; minimum 8.3** | Pending | Pending |
| Performance | 7.9 mean; minimum 7.0 (source review with scoped prior measurements) | **Provisional 8.40; minimum 8.0** | Pending | Pending |
| Design | 8.1 static mean; minimum 7.0 | External fixed-rubric source rescore 8.97, minimum 7.84; runtime/integration pending | External source rescore 9.65, minimum 8.77; runtime/integration pending | External source rescore 9.96, minimum 9.80; runtime/integration pending |
| UX | 7.4 mean; minimum 7.1 (source review + bounded baseline journeys) | **Provisional 8.30; minimum 7.9** | Pending | Pending |

Current C/P/UX checkpoint: [partition dimensions, deductions and execution evidence](iteration-4-provisional-scores.md). This is an evidence synthesis after repairs, not a completed same-reviewer final acceptance round. These scores precede the subsequent mounted recap repair and focused browser passes; their numerical judgments are preserved. Current acceptance and remaining integration/native/load gaps are recorded in VERIFICATION.md. The latest grouped backend batch passed 106/106; no prospective credit is awarded for unrun checks.

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

## Other iteration 4 partition baselines

| Partition | Performance | UX | Independent design |
|---|---:|---:|---:|
| 1 | 8.7 | 7.1 | 8.8 |
| 2 | 7.5 | 7.2 | 8.5 |
| 3 | 7.0 | 7.4 | 8.8 |
| 4 | 7.7 | 7.6 | 8.2 |
| 5 | 8.4 | 7.5 | 9.1 |

Reports are `iteration-4-<lens>-<partition>.md`. Category means remain pending until all five partition reports are available. The independent design scores are bounded feature reviews; Claude's ten-lens 8.1 mean is a different aggregation of overlapping design findings and must not be averaged together with these.

Performance mean = 7.86 (reported 7.9), minimum = 7.0. Six major and two minor new performance findings are source-sized; their actual improvements still require current-source measurements. Prior measurements contribute only where their workload maps to the reviewed partition.

UX mean = 7.36 (reported 7.4), minimum = 7.1. Partition4 credits 1.1 execution for its explicitly mapped prior workflow leave regression; partition5 credits 1.0 because named Assistant/Rooms passes do not substantially cover its broad matrix. New source findings remain unverified until the current repairs execute.

All twenty baseline roles are complete. Independent design feature-partition mean = 8.68 (reported 8.7), minimum = 8.2. This is an additional bounded source audit, not a replacement for Claude's ten-lens 8.1 baseline or rendered acceptance.

## Design iteration4 source rescore received

Claude persisted the same-reviewer rescore at branch fix/design-iter-4 commit52f95235, source reviewedb751ca6f: docs/reviews/design-20261005/SCORES.md and iteration-4/rescore. Reported mean8.96 versus8.10 baseline, minimum7.8. Tokens8.5/layout9.4/components7.8/states9.4 strict arithmetic (reviewer judgement9.2)/a11y9.6/responsive9.8/copy8.2/agent9.0/theming9.2/IA8.7. Root read the summary and raw state report and requested normalization: using9.2 states yields8.94 mean, while9.4 yields8.96. This small ledger inconsistency does not change the demonstrated source-review improvement, but must be resolved before final scoring. No screenshots/runtime acceptance claimed; Claude baseline e2e comparison still runs, expected heavy release05:35–05:45 local. These files are not integrated into our branch yet.

Normalization update: Claude separated strict arithmetic from reviewer judgement. Official fixed-rubric design mean is8.97 (states9.37; theming9.15), with judgement average8.94 shown separately. Root read the normalized ledger. Baseline8.10→8.97 is the comparable source-review progress; runtime/integration remain pending.

## Design iteration 5 source rescore received

Root read Claude’s committed ledger in claude_ade-design5 (HEAD49bdbd0f; reviewed source4cc239da): same ten reviewers, unchanged arithmetic rubric, mean9.65 (unrounded9.649), minimum8.77(copy). Scores:tokens9.77/layout9.60/components9.37/states9.77/a11y9.87/responsive9.87/copy8.77/agent9.90/theming9.77/IA9.80. Comparable progression8.10→8.97→9.65. PR80 is draft/stacked on79 per Claude; not integrated or runtime-accepted here. Copy-only edits at reserved sites authorized narrowly; backlinks failure state remains a behavioral repair. No C/P/UX rescore inferred.

## Design iteration 6 source rescore received

Root read the ledger in claude_ade-design6, scored source75f1565b, ledger7361ac71. Same reviewers and fixed rubric: **9.96 average, minimum9.80**. Components and copy9.80; states9.97; remaining seven lenses10.00. This meets the source-review target only. Runtime/browser acceptance and integration are pending; subsequent residual fixes were not credited. PR81 is draft and stacked on80. PR79 merged as28cd216cff0a377be05d5acad3fe7916e3591d28; not yet integrated here. No correctness/performance/UX uplift inferred.
