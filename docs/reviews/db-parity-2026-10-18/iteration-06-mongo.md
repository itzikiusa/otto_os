# Mongo parity — iteration 06: Routing and argument boundaries

Focused static review after iteration05, with distinct scenarios for this pass. No Cargo or repeat test execution. Prior iteration03 evidence (95 Mongo-focused units,13 live cases, distinct live and focused explain) is retained as historical proof, not a claim that later changes have passed. Current upgraded-toolchain gate belongs to coordinator.

| Lens | Score | Trace and evidence |
|---|---:|---|
| L01 | 9.7 | Traced count vs countDocuments variants, collection count with absent filter, cursor find.limit.count (legacy count ignores cursor limit), and unsupported count(true) fallback. Iteration05 semantic repair awaits driver gate. |
| L02 | 9.8 | Traced extra find/update options to parser failure, missing delete filter refusal, dotted collection separation, and unsupported chains to shell. JSON unknown fields fail explicitly; JSON does not become JavaScript. |
| L03 | 9.7 | Traced do/switch/declaration token routing, comment-prefix removal, native keyword-shaped keys, literal shell markers and trailing comments. UI RED/GREEN proofs exist; new Rust detector tests await gate. |
| L04 | 9.8 | Traced run_many stopping after first failure and preserving completed result labels; execution errors stay errors and cannot re-enter parse fallback. No command retry introduced. |

Scores deliberately remain below target where newly changed code is unexecuted. No universal parity claim. Only evidenced owned-source fix in these five passes was the round08 bounded cancellation change; reports and tests document pending verification. No commit/push or production data access.

## Final gate resolution

All pending Mongo checks passed on the final owned implementation:98 focused units,10 script-policy checks (overlapping),13 live cases and1 separate distinct live regression. See [current-head evidence](final-verification-mongo.md). Final L01–L04 scoped assessments: **9.8 each**; earlier scores above record the honest pre-gate checkpoint.
