# Mongo parity — iteration 09: Exports and database selection

Focused static review after iteration05, with distinct scenarios for this pass. No Cargo or repeat test execution. Prior iteration03 evidence (95 Mongo-focused units,13 live cases, distinct live and focused explain) is retained as historical proof, not a claim that later changes have passed. Current upgraded-toolchain gate belongs to coordinator.

| Lens | Score | Trace and evidence |
|---|---:|---|
| L01 | 9.7 | Traced native find/aggregate export allowlist and explicit rejection of count/distinct streaming exports. Unsupported shell export forms fail clearly rather than discard clauses. |
| L02 | 9.8 | Traced JSON/NDJSON per-document column refresh and typed nested BSON values. CSV/TSV retain documented first-document schema and readable representation. |
| L03 | 9.7 | Traced selected database through resolve_db and shell prelude without rewriting connection-string authSource; URI credentials remain in0600 temporary script, IPv6/encoded discrete params and SOCKS append preserved. No new remote SSH/TLS verification claimed. |
| L04 | 9.7 | Traced export cursor iteration and optional row cap, empty export header/finish, file-writer error propagation. Cancel bound from round08 awaits test; no further defect found. |

Scores deliberately remain below target where newly changed code is unexecuted. No universal parity claim. Only evidenced owned-source fix in these five passes was the round08 bounded cancellation change; reports and tests document pending verification. No commit/push or production data access.

## Final gate resolution

All pending Mongo checks passed on the final owned implementation:98 focused units,10 script-policy checks (overlapping),13 live cases and1 separate distinct live regression. See [current-head evidence](final-verification-mongo.md). Final L01–L04 scoped assessments: **9.8 each**; earlier scores above record the honest pre-gate checkpoint.
