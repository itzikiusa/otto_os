# Cloud UX — round 3, reviewer 8/10

Worktree `/Users/itziklavon/otto-ux-audit-20260925`. Owned AWS/Kubernetes UI. No shared-source edits, commits, subagents, real cloud mutation, account credentials, host configuration, or original-checkout changes.

## Result and verified scope

R1+R2 regressions independently **31/31 passed** before changes. This review goes deeper into populated monitoring workload/event/fleet workflows, S3 pagination and CSV/binary previews, and real-shaped CloudWatch charts with refresh recovery. Seven new confirmed defects repaired, plus workload-name spacing cleanup. Full final outcomes are appended below after completion; the report is not a blanket 9.5 sign-off for every cloud operation.

## Repaired findings

1. **P2 — monitoring workload details and most sorting headers were pointer-only.** `ui/src/modules/kubernetes/monitor/MonitorCluster.svelte:354,377`. Loaded rows lacked any native button to expand the pod/trend details; seven headers attached clicks directly to `<th>`. Red test found no keyboard action despite visible rows. Added native workload buttons with `aria-expanded`, native sortable-header buttons, and aria-sort on the converted columns. Enter expands, then Enter on Workload sorts billing before checkout.
2. **P2 — old event-class response replaced the selected filter.** `MonitorCluster.svelte:204`. Delay OOM, select Crash, release OOM: the Crash filter displayed the previous OOM workload. A request generation owns success/error/finally updates; leaving the section/cluster/window invalidates older results.
3. **P2 — old workload trends appeared under a newer workload.** `MonitorCluster.svelte:178`. Delay checkout's two series, expand billing, release checkout: billing's detail displayed checkout's metric. Trend generation invalidates old responses on expansion/collapse and context changes.
4. **P2 — failed monitoring trends silently became empty charts.** `MonitorCluster.svelte:435`. A synthetic 503 yielded “0 points” without failure/recovery. Explicit inline “Couldn't load trends”, actual failure details and Retry trends now recover to a synthetic successful response.
5. **P2 — last monitoring time-range control clipped on phone.** `MonitorCluster.svelte:274,288,464`. At 375px, 7d's right edge was 377.14px; screenshot showed its rounded end clipped. Phone window control now occupies its own body toolbar, while desktop retains the header location.
6. **P2 — monitoring event class painted into the resource identity on phone.** `MonitorCluster.svelte:730`. The old two-column mobile grid placed Crash in the 10px dot column, with its overflowing text painted across `default/checkout-api`. A box-only overlap assertion incorrectly passed; the corrected Range text-bounds assertion reproduced it. Event metadata now occupies distinct second-column tracks at narrow container widths (also applies to tablet split content), and identities/messages wrap rather than becoming inaccessible ellipses.
7. **P2 — fleet drill-down inaccessible by keyboard.** `ui/src/modules/kubernetes/monitor/MonitorFleet.svelte:579`. Workload/pod rows only reacted to pointer clicks. Added named native buttons for “Show pods for …” and “Show events for …”, preserving full-row pointer behavior. Enter drills workload→pod→events; Clear resets selection, and loaded overview KPIs/charts render.

No new API/WS contract or global component changed. Compiler output for both changed Svelte components has zero warnings. Converted physical text alignment in the touched cluster table to logical alignment. A 4px gap separates workload identity and kind, which previously ran together in the screenshots.

## Commands and outcomes

Commands run from worktree `ui/` with this exact prefix:

```sh
OTTO_E2E_SLOT=ux3cloud OTTO_E2E_PORT=7857 OTTO_E2E_PW_PORT=5357 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod
```

- Prior repair check: `npx playwright test e2e/desktop-ux-cloud.spec.ts e2e/desktop-ux-r2-cloud.spec.ts --project=desktop-browser --workers=2 --output=/tmp/otto-ux-r3-cloud-prior-results` — **31 passed (1.7m)**. Log `/tmp/otto-ux-r3-cloud-prior.log`.
- First new red: `npx playwright test e2e/desktop-ux-r3-cloud.spec.ts --project=desktop-browser --workers=2 --output=/tmp/otto-ux-r3-cloud-red-results` — 4 expected failures, 5 passed (1.1m). Log `/tmp/otto-ux-r3-cloud-red.log`. All four errors corresponded to visible product defects, not fixture failures.
- First green of those fixes: same R3 spec, output `/tmp/otto-ux-r3-cloud-green-results` — **9 passed (37.4s)**. Log `/tmp/otto-ux-r3-cloud-green.log`.
- Deeper AWS + phone: R3 spec `--grep 'warm-light-phone|S3 pagination|CloudWatch'`, output `/tmp/otto-ux-r3-cloud-deep-results` — 2 passed; phone 7d bounds failed as described. Log `/tmp/otto-ux-r3-cloud-deep.log`.
- Fleet keyboard red: R3 spec `--grep 'fleet keyboard|phone event classification'`, output `/tmp/otto-ux-r3-cloud-deep2-results` — fleet failed before native buttons. The phone test had not yet been added to that loaded suite. Log `/tmp/otto-ux-r3-cloud-deep2.log`.
- Stronger event text bounds red + fleet green: R3 spec `--grep 'phone event classification|fleet keyboard'`, output `/tmp/otto-ux-r3-cloud-overlap2-results` — event text-overlap failure; fleet passed (30.9s). Log `/tmp/otto-ux-r3-cloud-overlap2.log`. An earlier box-only event assertion passed; it was strengthened using `Range.getBoundingClientRect()`, not accepted as proof against visible overlap.
- `npx tsc -p tsconfig.e2e.json --noEmit` — exit 0, `/tmp/otto-ux-r3-cloud-tsc.log`.
- `node scripts/ui-guards.mjs` — exit 0, no ratchet regressions, `/tmp/otto-ux-r3-cloud-guards.log`. Did not alter baseline.
- Direct `svelte/compiler` compile of both modified files — `[]` warnings each, `/tmp/otto-ux-r3-cloud-svelte.log`.
- Scoped `git diff --check` — exit 0. Parent owns full npm check/build.
- One final-suite launch exited before webserver startup with only `Process from config.webServer exited early` (`/tmp/otto-ux-r3-cloud-final.log`). Confirmed both assigned ports had no listener, then reran; no overlapping Playwright processes were launched.

Every AWS/Kubernetes request is intercepted. The fixture rejects unspecified non-GET requests. Synthetic send/scale/import checks from earlier suites retain their own explicit intercepts; production credentials or remote writes were never used.

## Rendered evidence actually inspected

R3 ten monitoring captures (all viewed through `view_image`):

- `/tmp/otto-ux-r3-cloud-native-light-{workloads,events}.png` — 1440×900.
- `/tmp/otto-ux-r3-cloud-native-dark-{workloads,events}.png` — 1440×900.
- `/tmp/otto-ux-r3-cloud-warm-light-phone-{workloads,events}.png` — 375×812.
- `/tmp/otto-ux-r3-cloud-warm-dark-tablet-rtl-{workloads,events}.png` — 834×1112.
- `/tmp/otto-ux-r3-cloud-pro-dark-{workloads,events}.png` — 1440×900.

Also viewed `/tmp/otto-ux-r3-cloud-cloudwatch.png` (loaded EC2 CPU card), `/tmp/otto-ux-r3-cloud-fleet-desktop.png` (1280×800), `/tmp/otto-ux-r3-cloud-fleet-phone.png` (375×812), and freshly regenerated earlier-suite `/tmp/otto-ux-r2-cloud-warm-light-phone-monitor.png`, `/tmp/otto-ux-r2-cloud-pro-dark-sqs.png`, `/tmp/otto-ux-cloud-tablet-rtl-kubernetes-loaded.png` (1024×768). Final repaired phone/tablet captures are re-inspected after final tests below.

The prior suite independently reran drawer clamping and RTL pointer/keyboard resize, phone focus trap/restoration, namespace popup and resource-kind keys, S3/SQS pending selection races, service tab keys, restricted permission selection, metrics Retry, numeric validations, nested scale sheet, EKS/RDS fixtures, and the internally-scrolling phone monitor/SQS forms. All five scheme/theme variants ran.

## Scores and precise limits

Scores below describe inspected main UI behavior. Production writes and physical VoiceOver exclusions are environment limitations, not arbitrary score caps. No score was raised because of round number or the 9.5 target.

| Page / important variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive | Mean |
|---|---:|---:|---:|---:|---:|---:|
| AWS account/setup, prior regressions | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 | 9.50 |
| EC2/RDS detail + CloudWatch populated recovery | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 | 9.48 |
| S3 listing/pagination/CSV/binary/text preview | 9.5 | 9.5 | 9.4 | 9.5 | 9.4 | 9.46 |
| SQS loaded/send/permission tabs, five variants | 9.5 | 9.5 | 9.5 | 9.4 | 9.5 | 9.48 |
| Athena query/history + EKS import confirmation | 9.5 | 9.5 | 9.4 | 9.4 | 9.3 | 9.42 |
| Kubernetes console/details/logs + phone/tablet RTL | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 | 9.50 |
| Monitoring settings, earlier five-variant suite | 9.5 | 9.4 | 9.3 | 9.4 | 9.5 | 9.42 |
| Monitoring workload/event desktop, five-theme evidence | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 | 9.48 |
| Monitoring workload/event phone + tablet RTL | 9.4 | 9.5 | 9.4 | 9.5 | 9.4 | 9.44 |
| Fleet overview/table/event keyboard drill, light desktop/phone | 9.5 | 9.5 | 9.4 | 9.3 | 9.4 | 9.42 |

Remaining coverage for R4/R5: monitor overview landing cards and watchdog Insights reports/workspace changes; Fleet Requests and large-page loading, range/group radiogroup arrow semantics, alternate themes/RTL, live WS refresh ordering; authenticated setup failure/retry; S3 actual download cancellation; SQS pending send/redrive/purge and draft preservation; Athena cancellation/error + phone results; EKS import-success navigation; multi-series CloudWatch cross-resource switching; full log streaming/reconnect. Do not treat these as confirmed defects. Full font/contrast measurements, custom accents/reduced motion, native Tauri and physical VoiceOver also remain outside this pass. No confirmed feasible finding from this round is deliberately deferred.

## Final verified handoff

- Final combined Chromium command: `npx playwright test e2e/desktop-ux-r3-cloud.spec.ts e2e/desktop-ux-cloud.spec.ts e2e/desktop-ux-r2-cloud.spec.ts --project=desktop-browser --workers=2 --output=/tmp/otto-ux-r3-cloud-final-results` with the prefix above — **44 passed (2.3m)**, exit 0. Log `/tmp/otto-ux-r3-cloud-final2.log`; `.last-run.json` = `passed`, no failedTests.
- Final WebKit command: `npx playwright test e2e/desktop-ux-r3-cloud.spec.ts --grep 'loaded monitoring|phone event classification|fleet keyboard' --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-r3-cloud-webkit-results` — **7 passed (1.1m)**, exit 0. Log `/tmp/otto-ux-r3-cloud-webkit.log`; `.last-run.json` = `passed`, no failedTests. This runs real WebKit with all five explicit viewport/theme fixtures, plus the phone overlapping-text and fleet keyboard regressions.
- Re-inspected final repaired Chromium Warm phone workloads/events and Warm tablet RTL events after the 44-test run. Re-inspected final WebKit Warm phone events after its completed suite; the 1h/6h/24h/7d control fits, event text no longer overlaps, full identity/message remain readable. WebKit captures overwrite the same variant names at device pixel ratio 3; dimensions above are CSS viewport sizes.
- Fresh scoped diff check exit 0 after final tests. No source changes after these successful runs. Full npm/build gates and integration remain parent-owned.

Changed files: `ui/src/modules/kubernetes/monitor/MonitorCluster.svelte`, `ui/src/modules/kubernetes/monitor/MonitorFleet.svelte`, new `ui/e2e/desktop-ux-r3-cloud.spec.ts` (13 new cases). No shared-file coordination was necessary.
