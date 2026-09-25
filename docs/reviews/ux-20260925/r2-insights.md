# Round 2 — reviewer 9/10: Assistant, History, Usage, Insights / Health

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. No commits, subagents, original-checkout edits or shared-component changes. Runtime writes were isolated test state; outward Assistant/report operations were intercepted. History used a newly written synthetic JSONL transcript, temporary workspace and harness fake Claude CLI. Import responses were intercepted; the resumed session was the isolated fake-CLI session. Screenshots below contain only synthetic fixtures.

## Prior repairs independently rechecked

All nine cases from `ui/e2e/desktop-ux-insights.spec.ts` passed in the combined run: Assistant tab focus, profile edits during save, Insights filter/selection consistency and retry, long Memory tags at phone/RTL, History failed filtered search at phone width, Usage attribution retry/long-key CSV, approval-queue retry, and light/dark loaded Assistant chat/tasks/memory (including submission and decision transitions). Both existing `desktop-insights.spec.ts` cases also passed: populated KPI/action ledger/Markdown/sandboxed HTML and phone list/detail navigation.

## Confirmed findings repaired

| Severity | Surface | Reproduction and repair |
|---|---|---|
| P1 | `ui/src/modules/assistant/cards/NeedsYouCard.svelte:121` | Compact task approvals clipped the complete outgoing payload to a tooltip. Phone users now have a native disclosure, containing a named keyboard-focusable scroll region. Test reads the final payload line before an intercepted Send. |
| P1 | `ui/src/modules/assistant/MemoryTab.svelte:41` | Editing profile then choosing Tasks silently discarded the draft. Uses the existing unsaved-change router guard; Keep editing preserves text and save then navigation succeeds. |
| P2 | `ui/src/modules/assistant/AssistantPage.svelte:98` | In RTL, Left from Chat selected Permissions. Arrow traversal now derives physical direction from computed CSS; Left reaches Tasks and focus follows. |
| P2 | `ui/src/modules/insights/InsightsPage.svelte:185` | A direct phone report URL opened the list with its report detail hidden. Route selection now enters phone detail; Back clears the routed detail and reveals the list. An unresolved explicit report key is retained for the existing missing-report state rather than silently replaced. |
| P2 | `ui/src/modules/insights/InsightsPage.svelte:415` | Replacing an existing report period never completed polling because count stayed constant. Polling compares report identities/content, selects the changed report, clears cached Markdown, refreshes the ledger and releases Run now. Same-count replacement is exercised. |
| P2 | `ui/src/modules/insights/InsightsPage.svelte:56,508` | Reports/Health claimed tab semantics but lacked arrow/Home/End behavior. Added roving tabindex and keyboard selection/focus. Tested keyboard entry to Health, failed load/Retry, dependency expansion and support-bundle download. |
| P2 | `ui/src/modules/agents/history/HistoryPage.svelte:94` | Select an older conversation then Today: the old conversation remained open outside the visible list. Selection and deep-link matching now resolve against visible entries; the desktop chooses a visible replacement. |
| P2 | `ui/src/modules/insights/CapabilitiesPage.svelte:251` | Phone Health clipped “Agent Sessions” to “Ag…” while page-overflow checks passed. Phone cards now wrap the header and put Fix on a separate row; dependency details can wrap. Regression checks actual label clipping. |
| P2 | `ui/src/lib/api/usage.svelte.ts:259` | A delayed initial 30-day summary failure overwrote a newer successful 7-day / All selection. Initial-load errors now obey the same request sequence as successful responses. The test controls release of the stale failure. |
| P2 | `ui/src/modules/usage/UsagePage.svelte:924` | Typing a budget window left Save budgets disabled until the field blurred. Numeric budget inputs mark the draft dirty on input, allowing direct Save. |
| P1 | `ui/src/modules/usage/UsagePage.svelte:169` and `ui/src/lib/api/usage.svelte.ts:283` | Save 7 days, edit to 90 while PUT is pending: response restored 7. Save now returns an explicit outcome and clears dirty only when the current draft equals the submitted one. Failed saves and newer pending edits remain intact. |

No confirmed feasible defect remains deferred in this reviewed set. Shared focus changes were consumed, not edited. Parent-reported new Svelte warnings were resolved without suppressions: Insights tablist has tabindex=-1; payload scroll regions receive keyboard focus through a local action.

## Commands and outcomes

All successful Playwright commands ran from `.../otto-ux-audit-20260925/ui` with:

```sh
OTTO_E2E_SLOT=ux2insights OTTO_E2E_PORT=7848 OTTO_E2E_PW_PORT=5348 \
OTTO_E2E_SWEEP_ORPHANS=0 \
OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod \
npx playwright test <specs/options below> --project=desktop-browser --workers=1 --output=<output>
```

- Initial red: `desktop-ux-r2-insights.spec.ts`, output `/tmp/otto-ux-r2-insights-red`, log `/tmp/otto-ux-r2-insights-red.log`: expected missing-preview/guard/RTL/deep-link/polling/tab failures. History's first fixture omitted Transcript.stats and failed rendering; corrected to the actual DTO before accepting a History finding.
- Second: same spec, output `/tmp/otto-ux-r2-insights-second`, log with same stem `.log`: **6 passed, 2 failed**. History date selection failed as intended; budget Save remained disabled and exposed the input-versus-change issue.
- Deeper red: same spec `--grep 'stale initial|Health keyboard'`, output `/tmp/otto-ux-r2-insights-deeper-red`, same-stem log: **2 expected failures**, stale Usage error and internally clipped Health name.
- Budget red: same spec `--grep 'budget failed'`, output `/tmp/otto-ux-r2-insights-budget-red`, same-stem log: **1 expected failure**, newer 90-day draft reverted to 7.
- Combined verification: `desktop-ux-r2-insights.spec.ts desktop-ux-insights.spec.ts desktop-insights.spec.ts`, output `/tmp/otto-ux-r2-insights-final`, same-stem log: **24 passed, 1 failed**. Health's first responsive override appeared before base rules and was overridden; moved the media rules to the end.
- History/Health follow-up: same new spec `--grep 'Health keyboard|History real'`, output `/tmp/otto-ux-r2-insights-history`, same-stem log: **1 passed, 1 failed**. Real History transcript/import-retry/resume passed. Health run still reflected the intermediate CSS; final run below verifies the corrected source.
- **Final new suite:** `desktop-ux-r2-insights.spec.ts`, output `/tmp/otto-ux-r2-insights-green`, log `/tmp/otto-ux-r2-insights-green.log`: **15 passed (1.3m), exit 0**. `.last-run.json` says `status: passed`, empty `failedTests`.
- `npx tsc --noEmit -p tsconfig.e2e.json`: exit 0; logs `/tmp/otto-ux-r2-insights-tsc.log` and final `/tmp/otto-ux-r2-insights-tsc-final.log`.
- `node scripts/ui-guards.mjs`: exit 0; `/tmp/otto-ux-r2-insights-guards.log`. No baseline increases.
- Scoped `git diff --check`: exit 0.
- Parent full UI check: `/tmp/otto-ux-r2-final-check.log` reports **0 errors, 0 warnings**; parent owns this gate.

Two invocation-only mistakes did not exercise the app: an initial relative output path included `ui/` while already in `ui/`; a budget command ran from root and reported missing desktop-browser project. Corrected cwd/path, then reran; neither is represented as an app failure.

## Rendered evidence actually viewed

All paths under `/tmp/otto-ux-screenshots/`:

- `insights-r2-approval-phone.png`: disclosed full approval payload, keyboard-focus ring; enclosing Tasks page scrolls to Send.
- `insights-r2-health-phone.png`: corrected full module label, Fix action, expanded dependency, successful synthetic support-bundle feedback.
- `insights-r2-history-real-synthetic.png`: real server-parsed two-turn read-only conversation with resume controls.
- `insights-r2-usage.png`: loaded synthetic provider and 7-day summary after stale-error race.
- `insights-r2-memory-native-light-1440.png`, `insights-r2-memory-warm-light-390.png`.
- `insights-r2-reports-native-dark-1440.png`, `insights-r2-reports-warm-light-390.png`, `insights-r2-reports-warm-dark-1024.png`, `insights-r2-reports-pro-dark-dark-1440.png`: refreshed rich synthetic metrics/actions/summary fixtures; light/dark themes and RTL tablet checked.
- Independently regenerated first-round evidence viewed: `insights-r1-assistant-light-loaded.png`, `insights-r1-tasks-dark-loaded.png`.

## Scores

Scores describe rendered UX and the exercised workflows, not production-service availability. L/I/A/S/R = layout-readability / interaction / accessibility / states-recovery / responsiveness.

| Page or important variant | L | I | A | S | R | Mean |
|---|---:|---:|---:|---:|---:|---:|
| Assistant Chat, loaded light/dark | 9.5 | 9.4 | 9.4 | 9.4 | 9.5 | 9.44 |
| Assistant Tasks, phone complete approval + desktop dark | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 | 9.52 |
| Assistant Memory, desktop/phone/RTL | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 | 9.52 |
| History, actual loaded conversation + filtered phone error | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 | 9.48 |
| Usage, loaded phone attribution and desktop budget/race | 9.5 | 9.5 | 9.4 | 9.6 | 9.5 | 9.50 |
| Insights Reports, rich desktop / phone / Warm RTL / Pro Dark | 9.6 | 9.5 | 9.5 | 9.5 | 9.5 | 9.52 |
| Insights Health, loaded/error phone | 9.5 | 9.5 | 9.5 | 9.6 | 9.5 | 9.52 |

Chat is below 9.5 because this pass rechecked submission and loaded rendering but has not deeply exercised attachment handling, provider pinning, or switching threads during a pending response. History's low dimension reflects incomplete keyboard traversal/long paged-history verification; the formerly missing loaded/read/resume flow is now exercised. These are verification gaps, not confirmed defects deferred.

## Fresh round 3 targets

- Assistant: attachment upload/error/removal, provider-pinning failure/retry and thread switch while a turn loads; profile Discard branch/browser history as well as the Keep editing branch verified here.
- History: long multi-page conversations, keyboard list traversal, workspace switch while paginated load is pending, real on-disk import (this round intercepted import response while exercising real isolated resume).
- Usage: add/remove provider and workspace budget rows, validation/cap enforcement UI, slow save while a background budget refresh completes; attribution group changes under latency.
- Insights: delayed full Markdown fetch while the same period regenerates, generation timeout/retry and missing explicit report URL, more than one Health dependency and support-bundle failure retry.
- Native Tauri/VoiceOver and a physical WebKit phone were not run. Chromium viewport tests establish the inspected browser UI only. No score ceiling is imposed merely because external services were intercepted.
