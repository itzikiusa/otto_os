# Iteration 10 — root L21–L24: integrated regression review

Final source pass reconciled the native driver results, shared classifications,
editor routing, grid notices and published help. Mongo's trailing-comment read
classification and Redis's nested EXEC errors were failures of real integration
checks, not score-only review observations. Both were fixed and their focused
and live regressions now pass. MySQL retains wire completion boundaries even
when a server lexical mode differs from the editor's statement view.

The in-app help now describes native distinct, shell fallback without replaying
native errors, both ClickHouse cancellation transports and Redis connection
closure. Updated service commentary includes distinct/countDocuments. No API
payload changes require new TypeScript types.

| Lens | Assessment | Evidence |
|---|---:|---|
| L21 query routing | 9.8 | UI 1996 tests; Mongo script-policy10, native98 and live13+distinct1; MySQL32 live including mode/boundary regression. |
| L22 layout/accessibility | 9.8 | Four fresh desktop/phone light/dark browser cases; fixed bounding box, footer and final field reachable; prior keyboard case preserved. |
| L23 contracts/guidance | 9.8 | Reviewed all four driver export/run paths against feature/API/help text, with explicit limits. |
| L24 release | Pending CI/release | Full workspace fmt/clippy/tests/docs/UI and plugin gates passed. No merge or installed-app claim yet. |

These scores assess the reviewed application behavior, not complete database
language coverage. CH native JSON/Dynamic/Variant still require HTTP or an
explicit conversion. Redis continuous push sessions remain terminal workflows.
MySQL LOCAL host files are regular files up to 100 MiB; possible LOCAL root requests pay dedicated
connection setup. Mongo arbitrary JavaScript retains conservative write policy.

Final gate evidence and release status are consolidated in README.md and the
final verification report; earlier reports remain historical records.
