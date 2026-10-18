# Mongo parity — iteration 07: BSON and display bounds

Focused static review after iteration05, with distinct scenarios for this pass. No Cargo or repeat test execution. Prior iteration03 evidence (95 Mongo-focused units,13 live cases, distinct live and focused explain) is retained as historical proof, not a claim that later changes have passed. Current upgraded-toolchain gate belongs to coordinator.

| Lens | Score | Trace and evidence |
|---|---:|---|
| L01 | 9.7 | Traced distinct scalar shaping versus count scalar output, empty distinct column presence, and separate legacy count result parsing. Count integration remains pending. |
| L02 | 9.8 | Traced nested BSON documents/arrays, rare canonical sentinels, legacy UUID subtype03 preservation, nonfinite doubles and extreme dates. Existing roundtrip regression evidence retained. |
| L03 | 9.7 | Traced shell canonical EJSON output for scalar, document, array and cursor. Array/cursor cap+1 probe marks truncation; cursor closes in finally. No new shell execution in this pass. |
| L04 | 9.8 | Traced exact explicit find limit versus explorer cap, zero display cap, sparse key union/spill, byte and cell charging before retaining each row. Distinct still receives one server BSON reply; this documented limit is unchanged. |

Scores deliberately remain below target where newly changed code is unexecuted. No universal parity claim. Only evidenced owned-source fix in these five passes was the round08 bounded cancellation change; reports and tests document pending verification. No commit/push or production data access.

## Final gate resolution

All pending Mongo checks passed on the final owned implementation:98 focused units,10 script-policy checks (overlapping),13 live cases and1 separate distinct live regression. See [current-head evidence](final-verification-mongo.md). Final L01–L04 scoped assessments: **9.8 each**; earlier scores above record the honest pre-gate checkpoint.
