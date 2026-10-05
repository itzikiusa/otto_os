# Iteration 5+6 browser e2e — results and triage

**Not all green.** Desktop-browser Playwright suite (every `ui/e2e/desktop-*.spec.ts` except perf), run on `fix/design-iter-6` @7361ac71, 2 workers, isolated daemon (installed `ottod`), 2026-10-05 06:20–06:53:

| Result | Count |
|---|---|
| Passed | 1019 |
| Failed | 84 |
| Skipped | 82 |
| Did not run | 13 |
| Total | 1198 |

For comparison, iteration 4's identical run (`fix/design-iter-4` @b751ca6f) was 998 pass / 100 fail / 82 skipped / 18 did not run.

## Triage of the 84 failures

1. **Present on main too (70).** These failed in the iteration-4 run and in main-baseline runs: Docker-dependent Mongo/Redis editing, the Help tour film (no local film asset), the swarm/automation 120 s visual fixtures, proof-pack "database is locked", AWS localstack, and others. They are not caused by the design passes.
2. **Failed on this branch but not in the full-suite iteration-4 run (14).** Each was re-run on main @28cd216c (same 15 spec files, 2 workers: 86 pass / 6 fail) and on this branch (97 pass / 8 fail), then serially:
   - **Fixed (copy or role changes the specs still asserted):** Skills Lab duplicate name (curly quotes), the Activity panel "From board" badge (now `Badge`), Plugin "Reload {name}" → "Restart {name}", the coach skills retry (now `LoadState`), and coach "Installed".
   - **Fixed (test helper):** the phone/tablet cloud drawers were measured mid slide-in. `expectFullyInViewport` now waits for running entrance animations. All five cloud variants pass.
   - **Environmental under the full parallel run:** db-error-panel, db-layout-stability, db-multirun, workbench "Send to → Database", notification-bell count, r2-settings bulk roles and ux-shell "Needs you" all passed in the isolated reruns. Their failures in the full run were mock-MySQL "pool timed out" / "All connections are busy", or shared-state counts.
   - **Known risk (not fixed):** `desktop-db-query-builder.spec.ts` "round trip: Open in Builder…" fails on this branch whenever it runs in parallel with the other DB specs (mock MySQL pool timeout), but passes serially. Main passed it in parallel. A higher DB request rate from the new UI has not been ruled out.

Final serial re-run of the residual set: 16/17 pass, then 19/19 after the last two spec fixes (commit 52520431).

**Not covered:** WebKit (only the CI advisory subset runs it), real Tauri windows, and a fresh full-suite run after the e2e fixes.
