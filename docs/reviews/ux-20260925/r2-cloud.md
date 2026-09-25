# Cloud UX — round 2, reviewer 8/10

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. Scope: AWS and Kubernetes UI. No commits, subagents, shared-file edits, real cloud operations, or original-checkout edits.

## Result

All six R1 residuals are repaired. The seven original fixes remain independently green. New browser-reproduced S3/SQS races, phone clipping, SQS numeric validation and permission-tab keyboard defects are repaired. This is a scoped review, not a whole-app or full cloud-module 9.5 sign-off; dashboard/fleet and several secondary operations still need fresh coverage.

## Prior findings verified and repaired

- R1 suite independently passed **13/13** before changes using the current worktree release daemon (`/tmp/otto-ux-r2-cloud-r1.log`). Reverified again in the combined suite.
- **P2, oversized tablet drawer:** `ui/src/modules/kubernetes/ClusterWorkspace.svelte:101`. A stored 1500px drawer extended to x=1894 in a 1024px viewport. Width now derives from actual workspace width, reserving 280px for the resource list and 168px for kinds; smaller workspaces use a full-screen detail sheet. Tested oversized persistence, close/recovery, and tablet RTL; the normal 1024px capture now shows a useful ~280px list beside a ~350px drawer.
- **P2, RTL / keyboard splitter:** `ClusterWorkspace.svelte:109`. A rightward RTL drag reduced width from ~519 to ~477px. Pointer delta now follows direction, arrows/Home/End resize within bounds, and a native range input exposes the value without new accessibility suppressions. Direct Svelte compile: no warnings.
- **P2, RTL logs:** `ui/src/modules/kubernetes/LogsView.svelte:269`. Browser saw computed RTL on log lines. The log data container now explicitly uses LTR; chrome remains RTL.
- **P2, AWS phone keyboard focus:** `ui/src/modules/aws/AwsDrawer.svelte:38`. Reused the existing shared `dialogFocus` action for focus containment, restoration, nested-sheet ownership and Escape. The initial Enter-open regression could close immediately under the old microtask focusing; the shared frame-timed focus path passes. Kubernetes `ResourceDrawer.svelte:191` also uses the shared behavior; this addresses actual Safari pointer restoration and nested dialogs, not merely duplication. Shared source was not edited.
- **P2, SQS / Athena tab keys:** `ui/src/modules/aws/SqsView.svelte:307`, `AthenaView.svelte:453`, `util.ts:112`. Named tab lists, roving tabindex and direction-aware arrows/Home/End activate and focus enabled tabs. Tested Messages→Send→Redrive→Messages and Athena Results→History.
- **P3, metrics recovery:** `ui/src/modules/kubernetes/MetricsView.svelte:56`. Named inline failure and Retry now recover from a synthetic 503 without waiting for the polling interval.

## Additional repaired findings

- **P2 — SQS stale Peek crossed queue selection**, `SqsView.svelte:75,96`. Hold queue A's response, select B, release A: A's payload appeared under B. Each selection/request now advances a version; old success/error/finally paths cannot replace B's state. The final regression waits for the released response and a browser frame before asserting absence.
- **P2 — S3 stale folder listing replaced current navigation**, `ui/src/modules/aws/S3Browser.svelte:89`. Hold `old/`, navigate to `new/`, release old: the row title reverted to `old/readme.txt`. Superseded responses are ignored. Preview failure now offers **Retry preview** in place; synthetic 503→text success is verified.
- **P2 — phone AWS primary action clipped**, `ui/src/modules/aws/AwsPage.svelte:165`. Actual screenshot and bounds showed Add account reaching x≈386 at 375px. On phones it is a named, titled + action; desktop keeps the text. All five variant assertions verify its bounds.
- **P3 — phone SQS attribute remove controls overflowed**, `SqsView.svelte:694`. Actual screenshot showed the rightmost remove buttons cut at the viewport. Grid tracks now permit shrinking and inputs use min-width:0; six long-form rows and last remove control are verified. Dynamic name/value inputs now have distinct accessible labels.
- **P2 — invalid SQS delay could be sent**, `SqsView.svelte:147`. Delay 901 left Send enabled because the form uses click submission. A whole-number 0..900 check gates both handler and button and gives inline text; synthetic successful send checks the actual request body after correction.
- **P2 — browse-only SQS lost its tab-stop**, `SqsView.svelte:47`. With enforced sqs_view but no receive/send/redrive, Messages was disabled and every enabled tab had tabindex=-1. A loaded permission change now moves an unavailable selected tab to Attributes; keyboard navigation skips disabled tabs.
- **P3 — RTL message/code fields reversed punctuation**, `SqsView.svelte` message/compose regions and `ui/src/modules/kubernetes/monitor/MonitorSettings.svelte` `.mono`. Visually verified JSON punctuation at the wrong edge and `/metrics` displayed as `metrics/`. Data regions now stay LTR; tests assert actual computed direction under tablet RTL.

## Verification and evidence

Commands run from worktree `ui/`, prefixed with:

```sh
OTTO_E2E_SLOT=ux2cloud OTTO_E2E_PORT=7846 OTTO_E2E_PW_PORT=5346 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- Baseline: `npx playwright test e2e/desktop-ux-cloud.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-cloud-r1-results` — **13 passed (58.6s)**. Log `/tmp/otto-ux-r2-cloud-r1.log`.
- Combined verification: `npx playwright test e2e/desktop-ux-r2-cloud.spec.ts e2e/desktop-ux-cloud.spec.ts --project=desktop-browser --workers=2 --output=/tmp/otto-ux-r2-cloud-results` — **30 passed (1.3m)**. Log `/tmp/otto-ux-r2-cloud-final.log`. Before the final restricted-permission/RTL-field additions.
- Final R2 and WebKit results appended below after completion.
- `npx tsc -p tsconfig.e2e.json --noEmit` — exit 0; `/tmp/otto-ux-r2-cloud-tsc.log`.
- `node scripts/ui-guards.mjs` — exit 0, no ratchet regressions; `/tmp/otto-ux-r2-cloud-guards.log`.
- Direct Svelte compiler pass over all ten changed Svelte files — no warnings; `/tmp/otto-ux-r2-cloud-svelte.log`.
- Scoped `git diff --check` passed. Full npm check is parent-owned.

Expected red evidence lives in `/tmp/otto-ux-r2-cloud-{red,red2,deep2,permission-red}.log` and corresponding results directories. Some initial test failures were fixture/test assumptions: FIFO suffix belongs to the cell's accessible name; validation text appeared both inline and in the toast; the scale action includes an ellipsis; initial Pro Dark fixture used `pro` and was corrected to actual `pro-dark`; S3 racing test now waits until the first request exists. One interim long run overlapped file changes and was rerun against stable source. None of those was represented as a product defect.

Every AWS/K8s request in the new suite is intercepted. Default non-GET requests are rejected; specific tested successful operations are fulfilled synthetically. No AWS account, cluster or production write was made.

### Main flows covered

AWS account/setup source keys and validation; all six service routes; EC2 detail overview/metrics/raw; phone detail keyboard entry, trap and restoration; S3 listing failure→Retry, prefix race and preview recovery; populated FIFO/standard SQS queues, tab keys, attributes, successful synthetic send, delay validation, queue-switch race, restricted permissions; Athena synthetic execution→result→history→same result; populated EKS nodegroups/import confirmation cancellation and RDS detail/raw JSON. Kubernetes kind/namespace/table navigation, long pod details, searchable logs, namespace popup bounds, tab keys, metric failure/retry, large persisted drawer widths, RTL pointer/keyboard resize, phone/nested scale validation and focus restoration. Monitoring settings render/load, empty/out-of-range validation, add probe, and scrolling of long forms.

### Actual screenshots viewed

All ten final R2 form captures use synthetic service data:

`/tmp/otto-ux-r2-cloud-{native-light,native-dark,warm-light-phone,warm-dark-tablet-rtl,pro-dark}-{sqs,monitor}.png`

Also viewed `/tmp/otto-ux-r2-cloud-athena-result.png`, `/tmp/otto-ux-cloud-tablet-rtl-kubernetes-loaded.png`, and before/after phone SQS captures. R1 suite regenerated loaded AWS/Kubernetes native-light/native-dark/warm-dark/phone-light/tablet-RTL captures. The initial invalid `pro` theme capture was replaced and is not evidence for Pro Dark; the final `pro-dark` capture was actually viewed.

## Scores (out of 10, inspected page family/variant)

| Page / important variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive | Mean |
|---|---:|---:|---:|---:|---:|---:|
| AWS account/setup | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 | 9.50 |
| EC2 / RDS loaded detail + phone sheet | 9.5 | 9.5 | 9.5 | 9.4 | 9.5 | 9.48 |
| S3 objects / prefix / text preview | 9.4 | 9.5 | 9.3 | 9.5 | 9.4 | 9.42 |
| SQS populated queues / send / permission tabs | 9.5 | 9.5 | 9.5 | 9.4 | 9.5 | 9.48 |
| Athena workbench / results / history | 9.5 | 9.5 | 9.4 | 9.4 | 9.3 | 9.42 |
| EKS detail / import confirmation | 9.5 | 9.5 | 9.4 | 9.3 | 9.3 | 9.40 |
| Kubernetes pod/table/detail/logs + tablet RTL | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 | 9.50 |
| Kubernetes nested workload scale sheet | 9.5 | 9.5 | 9.5 | 9.4 | 9.5 | 9.48 |
| Monitoring settings: native light/dark + Pro Dark | 9.5 | 9.4 | 9.3 | 9.4 | 9.5 | 9.42 |
| Monitoring settings: Warm phone / tablet RTL | 9.4 | 9.4 | 9.3 | 9.4 | 9.4 | 9.38 |

The sub-9.5 scores reflect narrower evidence for secondary behaviors: S3 CSV/binary/download and large pagination; full populated CloudWatch charts and their interaction; Athena cancellation/error and small-phone result grids; EKS import-success navigation; SQS delete/redrive/purge confirmations; complex JSON probe mappings/Test probes; and complete keyboard traversal across those panels. These are verification limits, not asserted defects. Fleet, overview/workload monitoring charts and the insights subview were not graded in this round. They need a loaded, synthetic, screenshot-backed pass. No score was reduced merely because production writes or physical VoiceOver were intentionally excluded.

No confirmed feasible finding from this review is knowingly deferred. Browser fixtures establish the inspected UI behavior; native Tauri, physical VoiceOver and real service integration remain outside this run.

## Final completion evidence

- Current R2 source, including restricted-permission and RTL code fields: `npx playwright test e2e/desktop-ux-r2-cloud.spec.ts --project=desktop-browser --workers=2 --output=/tmp/otto-ux-r2-cloud-final-results` — **18 passed (56.1s)**. Log `/tmp/otto-ux-r2-cloud-final2.log`; `.last-run.json` is `passed` with no failedTests.
- Actual WebKit iPhone: `npx playwright test e2e/desktop-ux-r2-cloud.spec.ts --grep 'AWS phone|warm-light-phone|nested sheets|browse-only' --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-r2-cloud-webkit-results` — **4 passed (16.2s)**. Log `/tmp/otto-ux-r2-cloud-webkit.log`; `.last-run.json` is `passed`. Exercises keyboard entry/trapping/return, long forms in Warm light, nested scale-sheet focus and restricted SQS tab navigation.
- Fresh scoped diff check passed after all edits. Final changed Svelte compiler output remains warning-free, E2E TypeScript and UI guards pass. Parent owns global npm/build gates and integration.
- Final tablet RTL SQS/monitor captures were viewed again after the LTR data-field changes. WebKit Warm-light phone captures were also viewed. All confirmed defects in this report have a repaired, verified path; remaining named coverage is for subsequent fresh rounds.

### Late visual finding: monitoring phone form overflow

The first WebKit screenshot showed a horizontal scrollbar despite a green document-overflow assertion. Added direct `.mon`/`.settings`/card/probe scroll-width checks; they reproduced **25–26px internal overflow** in the monitor settings card. **P2:** `ui/src/modules/kubernetes/monitor/MonitorSettings.svelte:439` action rows did not wrap, so the first card's action group extended beyond the phone content width. Rows now wrap. The original passing four WebKit cases are therefore supplemented by a stronger final forms check, not presented as proof that the original form had no clipping. Red log: `/tmp/otto-ux-r2-cloud-overflow.log`.

Final affected-form command: `npx playwright test e2e/desktop-ux-r2-cloud.spec.ts --grep 'loaded SQS' --project=desktop-browser --project=iphone-portrait --workers=2 --output=/tmp/otto-ux-r2-cloud-form-final-results` — **10 passed (26.4s)**. Same ux2cloud environment prefix. Log `/tmp/otto-ux-r2-cloud-form-final.log`, `.last-run.json=passed`. All five theme/device fixtures ran in both engines, now checking internal clipping. Final monitor Svelte compile `[]`, fresh tsc and guards exit 0, and scoped diff-check passes. Viewed repaired Warm phone monitor screenshot after these checks.
