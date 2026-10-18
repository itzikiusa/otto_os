# Mongo parity — iteration 08: Cancellation and error behavior

Focused static review after iteration05, with distinct scenarios for this pass. No Cargo or repeat test execution. Prior iteration03 evidence (95 Mongo-focused units,13 live cases, distinct live and focused explain) is retained as historical proof, not a claim that later changes have passed. Current upgraded-toolchain gate belongs to coordinator.

| Lens | Score | Trace and evidence |
|---|---:|---|
| L01 | 9.7 | Legacy count command explicitly carries maxTimeMS and unique server marker; countDocuments uses CountOptions. Unit command-shape and live semantic tests pending. |
| L02 | 9.8 | Traced negative/fractional/nonfinite/string count reply n rejection and exact integer conversion; parser errors never silently become successful empty counts. |
| L03 | 9.7 | Traced script timeout and kill_on_drop, nonzero shell status with bounded diagnostic tails, and missing binary install guidance. Raw scripts retain print behavior. |
| L04 | 9.7 | Confirmed unbounded cancel control path could postpone service abort indefinitely while connect/currentOp/killOp stalled. Repaired entire control sequence with5sec timeout returning Error. New helper test covers pending future timeout, success, and original error propagation; existing live cancellation isolation retained. Execution pending. |

Scores deliberately remain below target where newly changed code is unexecuted. No universal parity claim. Only evidenced owned-source fix in these five passes was the round08 bounded cancellation change; reports and tests document pending verification. No commit/push or production data access.

## Final gate resolution

All pending Mongo checks passed on the final owned implementation:98 focused units,10 script-policy checks (overlapping),13 live cases and1 separate distinct live regression. See [current-head evidence](final-verification-mongo.md). Final L01–L04 scoped assessments: **9.8 each**; earlier scores above record the honest pre-gate checkpoint.
