# Mongo parity — iteration 10: Final regression alignment

Focused static review after iteration05, with distinct scenarios for this pass. No Cargo or repeat test execution. Prior iteration03 evidence (95 Mongo-focused units,13 live cases, distinct live and focused explain) is retained as historical proof, not a claim that later changes have passed. Current upgraded-toolchain gate belongs to coordinator.

| Lens | Score | Trace and evidence |
|---|---:|---|
| L01 | 9.7 | Re-read parser variants, executor match, JSON allowlist and read-only classification for new CountDocuments. All map consistently; prior normal-count tests plus new differing-semantics regression cover intended split. |
| L02 | 9.8 | Re-read numeric reply unit cases, BSON roundtrip and strict-options regressions against implementation. No new mismatch found; current build must verify exhaustive matches. |
| L03 | 9.7 | Re-read mirrored UI/backend tokens, comments and anchored special expressions with same fixture corpus. UI9/9 executed in iteration05; current backend/live checks remain pending. |
| L04 | 9.7 | Re-read service native-cancel error branch: timeout Error reaches task abort; inline governed execution surfaces error instead of fake success. Existing concurrent repeated-query-id live test remains in suite. New timeout helper and current integration gate pending. |

Scores deliberately remain below target where newly changed code is unexecuted. No universal parity claim. Only evidenced owned-source fix in these five passes was the round08 bounded cancellation change; reports and tests document pending verification. No commit/push or production data access.

## Current integration gate follow-up

Coordinator's full library run exposed `mongosh_markers_in_native_values_are_not_scripts` failing on a native find followed by a line comment. Script detector correctly returned false; the subsequent read-policy assertion failed because parse_shorthand required a dot after the closing method parenthesis and did not accept comment trivia. Repaired native parser advancement to skip line/block comments and whitespace. Unterminated block comments and executable trailing tokens still error. Added positive trailing/between-chain comment cases and negative malformed/mutating tails. Current Mongo focused/live verification is queued after other owners release Cargo; no passing claim yet.

## Final gate resolution

All pending Mongo checks passed on the final owned implementation:98 focused units,10 script-policy checks (overlapping),13 live cases and1 separate distinct live regression. See [current-head evidence](final-verification-mongo.md). Final L01–L04 scoped assessments: **9.8 each**; earlier scores above record the honest pre-gate checkpoint.
