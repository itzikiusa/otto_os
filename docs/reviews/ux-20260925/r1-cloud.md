# Cloud UX review — reviewer 7/10, round 1

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. Scope: AWS / Kubernetes. Date: 2026-09-25.

## Result

**Round complete; 13/13 focused browser tests pass. This is not a 9.5 sign-off.** Confirmed keyboard and phone defects were repaired. Remaining tablet/RTL issues and deeper service/monitoring coverage warrant another round.

| Dimension | AWS | Kubernetes | Combined |
| --- | ---: | ---: | ---: |
| Layout | 8.8 | 8.4 | 8.6 |
| Interaction | 8.7 | 8.7 | 8.7 |
| Accessibility | 8.2 | 8.3 | 8.25 |
| States | 8.7 | 8.4 | 8.55 |
| Responsive | 8.6 | 7.9 | 8.25 |
| Mean / 10 | **8.60** | **8.34** | **8.47** |

Scores include verified deficiencies and explicitly incomplete evidence. They are not assertions that every service action, tab or theme meets accessibility thresholds.

## Evidence and methods

Read AGENTS.md and design README/checklist/accessibility/layout guidance; used systematic-debugging and test-driven-development. Inspected all ten supplied AWS/Kubernetes screenshots in native light/dark, Warm dark, phone light, tablet RTL. The original AWS native screenshots were still loading, so they did not demonstrate loaded account views.

Created `ui/e2e/desktop-ux-cloud.spec.ts`, intercepting all AWS and Kubernetes requests plus resource capability responses. Every non-GET cloud request is blocked by the fixture; no real cloud mutation or credential operation ran. The authenticated app shell uses the isolated harness daemon.

Browser-verified:
- AWS six service routes: S3/SQS/EC2/Athena/EKS/RDS; EC2 loaded instance/detail and tab navigation; S3 failed refresh followed by successful Retry.
- AWS setup: credential-source keyboard selection, disabled Next before required credentials, disabled Save before name; sheet in viewport.
- Kubernetes: loaded long-name pod/table/drawer; Enter on resource-kind navigation; 90-namespace popup bounds; Tab closes popup; Enter selection retains focus; detail arrows advance focus twice.
- Kubernetes setup: source arrows, empty YAML validation, End to EKS and visible Go to AWS action.
- Searchable pod logs and loaded EC2 detail across five variants: 1440×900 native light, native dark and Warm dark; 375×812 phone light; 1024×768 tablet RTL. No document horizontal overflow; close buttons in viewport.
- Kubernetes phone drawer: modal semantics, hit testing at bottom edge above navigation, Shift+Tab containment, Escape closes.

Final command (from worktree `ui/`):

```sh
OTTO_E2E_SLOT=uxcloud OTTO_E2E_PORT=7818 OTTO_E2E_PW_PORT=5318 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/claude_ade/target/debug/ottod npx playwright test e2e/desktop-ux-cloud.spec.ts --project=desktop-browser --workers=2 --output=/tmp/otto-ux-cloud-results
```

Result: **13 passed (1.9m)**, exit 0. `npx tsc --project tsconfig.e2e.json --noEmit` passed; scoped `git diff --check` passed. Full `npm run check` belongs to the parent and was not run here.

Earlier red runs reproduced the defects below. Two earlier theme tests timed out during initial shell connection under concurrent machine load; both passed on rerun and in the final complete suite. An initial AWS fixture omitted required `status.install`; the fixture was corrected. Service smoke assertions were corrected for Athena's workbench layout and explicit refresh of an already cached bucket list; those were test assumptions, not product defects.

## Repaired findings

1. **P2 — Kubernetes Enter intercepted focused controls.** `ui/src/modules/kubernetes/ClusterWorkspace.svelte:332`. Repro: load pod rows, focus Deployments, press Enter. Previously opened the selected/first pod instead of navigating to Deployments. Global handler now leaves native buttons/links/tabs alone. Red: URL changed to pod detail; green: `/deployments`.
2. **P2 — Namespace popup survived keyboard departure and discarded selection focus.** `ui/src/modules/kubernetes/NamespacePicker.svelte:85,190`, `ClusterWorkspace.svelte:259`. Repro: focus Namespace, Tab; popup stayed above unrelated controls. Selecting a namespace also blurred the field. Blur now closes the popup; selection preserves input focus, and workspace Escape defers to the expanded combobox. Browser red/green includes 90 options and viewport bound check.
3. **P2 — AWS detail tabs stranded focus.** `ui/src/modules/aws/AwsDrawer.svelte:54`. Repro: focus Overview and press Right twice; selection changed to Metrics once but focus remained on Overview. Selection and focus now move together; Home/End and RTL arrow direction are implemented. Browser red/green reaches Metrics then Raw JSON.
4. **P2 — Kubernetes detail tabs stranded focus.** `ui/src/modules/kubernetes/ResourceDrawer.svelte:221`. Same root cause; browser red/green reaches Manifest then Describe. Same Home/End and RTL behavior added.
5. **P2 — AWS setup source tabs ignored arrow navigation.** `ui/src/modules/aws/AccountWizard.svelte:169,194`. Repro: focused profile tab + Right did nothing. Added roving tabindex, arrow/Home/End navigation with RTL direction and activation. Red/green verifies the access-key form and required-field gating.
6. **P2 — Kubernetes setup source tabs ignored arrow navigation.** `ui/src/modules/kubernetes/ClusterWizard.svelte:183,203`. Added the equivalent behavior. Red/green verifies Paste kubeconfig and End to EKS.
7. **P2 — Phone Kubernetes details painted under BottomNav and lacked modal focus behavior.** `ui/src/modules/kubernetes/ClusterWorkspace.svelte:705`, `ResourceDrawer.svelte:186,235`. Supplied loaded phone capture showed BottomNav covering the last part of the full-screen details. Drawer now uses `--z-modal`, registers its overlay, exposes dialog semantics, focuses Close, contains Tab, restores trigger focus and owns Escape while respecting nested modals. Browser red lacked dialog role; final green additionally proves hit testing at y=790, focus containment and closing. Updated phone screenshot shows the log footer instead of overlaid navigation.

## Residual findings for a fresh round

These remain deliberately visible in the score. Do not interpret the green regression suite as blanket coverage.

| Severity / evidence | Location | Repro / consequence | Next remedy |
| --- | --- | --- | --- |
| P2, browser visual verified | `ui/src/modules/kubernetes/ClusterWorkspace.svelte:449` and width setup near `:86` | At 1024px with sidebar and a 520px drawer, only roughly 110px remains for resource rows; tablet RTL screenshot shows almost only pod-name fragments. Document overflow passes, but comparing resources is cumbersome. Persisted wide drawers can worsen this. | Derive/clamp width from current workspace width, or switch to a detail overlay/push layout when two useful panes cannot fit. Test resize and reopening persisted widths. |
| P2, browser visual + source verified | `ui/src/modules/kubernetes/LogsView.svelte:269` | Tablet RTL log lines inherit RTL alignment/direction. Code/log punctuation should retain LTR ordering. | Put LTR direction on the data/log region and test punctuation-heavy lines, keeping surrounding chrome RTL. |
| P2, source traced; not browser-reproduced this round | `ui/src/modules/aws/AwsDrawer.svelte:36,43` | Phone AWS drawer focuses Close and declares a modal, but only handles Escape: no Tab containment and no saved trigger focus restoration. | Add a phone keyboard regression analogous to Kubernetes and implement/restyle through a shared drawer primitive if appropriate. |
| P2, source traced | `ui/src/modules/aws/SqsView.svelte:297`, `AthenaView.svelte:453` | These remaining role=tablist controls have click-only selection, no arrow/Home/End handling or roving tabindex. | Seed a real queue/query history in browser fixtures; repair tabs and verify keyboard access to every permitted panel, including disabled permission states. |
| P2, source traced | `ui/src/modules/kubernetes/ClusterWorkspace.svelte:102` | Splitter always subtracts pointer delta because it assumes drawer sits right. RTL places it left; pointer resizing direction is inverted. Separator is also not keyboard focusable. | Add RTL drag and keyboard-resize tests, direction-aware delta, bounded keyboard controls. |
| P3, source traced | `ui/src/modules/kubernetes/MetricsView.svelte:55` | A metrics failure only shows raw error text; retry relies on the 10s polling timer. | Add named inline failure text and explicit Retry; test recovery. |

Coverage still needed: loaded SQS messages/send/redrive/attributes, S3 objects and preview/download, Athena executed/results/history states, EKS import and RDS populated detail, Kubernetes workloads/exec/scale/sync/multi-pod logs and monitoring dashboards/settings/fleet. Destructive paths should remain intercepted, and confirmation/content validation can be checked without approving real writes. Axe/contrast, screen-reader and full keyboard traversal of those deeper views were not completed in this focused round.

## Artifacts

- Tests: `ui/e2e/desktop-ux-cloud.spec.ts`.
- Ten loaded captures: `/tmp/otto-ux-cloud-{native-light,native-dark,warm-dark,phone-light,tablet-rtl}-{aws,kubernetes}-loaded.png`.
- Final Playwright output: `/tmp/otto-ux-cloud-results`; HTML report: `ui/e2e/.report-uxcloud`.
- Product changes: AWS AccountWizard/AwsDrawer; Kubernetes ClusterWizard/ClusterWorkspace/NamespacePicker/ResourceDrawer. No commits, original-checkout changes, spawned agents, or external cloud changes.

Coordination note for round 2: the shell reviewer introduced shared `dialogFocus.ts` during this round. `ResourceDrawer.svelte:186–219` currently contains about 35 lines of local phone focus management; consider reusing that shared action after reviewer 10 verifies its behavior. This can also address the source-traced AWS phone drawer gap without multiplying local focus-trap implementations. No additional refactor was made after the final green run.
