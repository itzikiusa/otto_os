# Round 1 — reviewers 8/10: Assistant, History, Usage, Insights

Workspace: `/Users/itziklavon/otto-ux-audit-20260925`. No commits, shared-store changes, production data changes, or original checkout edits.

## Confirmed defects and repairs

1. **P1 — Profile edits lost during save.** `ui/src/modules/assistant/MemoryTab.svelte:43`: type A, Save, then type B before the PUT returns. The old response clears the entire draft and restores A. Browser red assertion received `- First edit` instead of `- A newer edit during save`. The save now captures submitted text and clears only an unchanged draft; duplicate shortcut submissions are blocked while saving.
2. **P2 — Assistant tab keyboard focus stranded.** `ui/src/modules/assistant/AssistantPage.svelte:95`: focus Chat, press Right; Tasks becomes selected but Chat keeps focus. The old callback reads event.currentTarget after dispatch; waiting for a Svelte tick also predates this router's hashchange. The handler now focuses the known destination button synchronously, independent of router scheduling.
3. **P2 — Insights filter and detail disagree.** `ui/src/modules/insights/InsightsPage.svelte:174`: open daily then select Weekly. The sole visible weekly row has no selection and the daily detail remains open. Selection now resolves against the filtered collection; an explicit filter clears a conflicting deep-link route.
4. **P2 — History filtered-load failure concealed on phone.** `ui/src/modules/agents/history/HistoryPage.svelte:392`: phone, search with no matches, then search again with a 503 response. Error/Retry lived only in the hidden detail pane because list error required existing rows; the list misleadingly said nothing matched. The list now presents error/Retry even with zero rows and suppresses the no-match claim while error exists.

5. **P2 — Tasks hides failed decision queue.** `ui/src/modules/assistant/TasksTab.svelte:59`: fail `/assistant/needs-you` while `/assistant/tasks` succeeds on phone. Running items render, but requests requiring a decision disappear without an error or Retry. An independent inline alert now exposes this failure and retries only the failed source.

## Evidence

Initial run `/tmp/otto-ux-insights-results`: 3 expected failures (profile draft, tab focus, filtered report selection), long memory tags phone/RTL passed. Second run `/tmp/otto-ux-insights-results-green`: profile, filtered selection, metadata and Usage passed; History error reproduction failed as expected; first focus repair was insufficient and was revised after tracing router hashchange scheduling.

Focused regression file: `ui/e2e/desktop-ux-insights.spec.ts`. Mocked backend boundaries run against an isolated daemon, not real sessions. Usage verifies populated provider/day summary, attribution failure/retry, long keys at phone width and CSV download filename. Memory checks delayed PUT and phone/RTL long metadata. Assistant loaded scenarios exercise composer submission and task decision transitions. Existing Insights report suite passed desktop KPI/ledger/Markdown/sandboxed HTML and phone list/detail navigation checks in the final run.

Screenshots inspected: initial native-light Assistant/History, native-dark Usage, warm-dark Insights, phone-light Assistant/History, tablet-RTL Insights/Usage. Loaded after screenshots use `/tmp/otto-ux-screenshots/insights-r1-*`.

Final full run: **11 passed (2.2m)**, exit 0, in `/tmp/otto-ux-insights-results-final` (9 focused cases plus 2 existing report render/navigation cases). The isolated environment used `OTTO_E2E_SLOT=uxinsights`, daemon port 7819, Vite port 5319, supplied debug binary, and orphan sweeping disabled. Final fixture-only rerun with canonical ISO dates: **1 passed (53.8s)**, exit 0, output `/tmp/otto-ux-insights-results-fixture`. This refreshes the report screenshot with realistic date labels. Playwright HTML report: `ui/e2e/.report-uxinsights`; retained failure traces/screenshots remain under the three earlier output directories noted above.

## Scores and remaining work

Post-repair, evidence-limited scores. Scale: layout / interaction / accessibility / states / responsive; mean out of 10.

| Page | Layout | Interaction | Accessibility | States | Responsive | Mean |
|---|---:|---:|---:|---:|---:|---:|
| Assistant Chat | 9.0 | 9.0 | 8.7 | 8.8 | 9.0 | 8.9 |
| Assistant Tasks | 9.0 | 8.8 | 8.7 | 9.0 | 9.0 | 8.9 |
| Assistant Memory | 9.0 | 9.2 | 8.7 | 8.8 | 9.0 | 8.9 |
| History | 8.8 | 8.5 | 8.6 | 8.8 | 9.0 | 8.7 |
| Usage | 9.0 | 9.0 | 8.7 | 9.0 | 9.1 | 9.0 |
| Insights | 9.0 | 9.0 | 8.2 | 8.9 | 9.0 | 8.8 |

**Overall mean: 8.9/10.** These are not 9.5 ratings: live external agent work, VoiceOver, custom accent contrast, and all significant failure paths were not verified.

Specific next-round targets:

- **Assistant Tasks, high priority:** `ui/src/modules/assistant/cards/NeedsYouCard.svelte:118` clips the exact outbound payload to one line in compact mode; full text is only a `title` tooltip. Loaded light/dark screenshots show the truncation. Phone users cannot inspect this tooltip before Send. Add an accessible inline disclosure or full preview, then test review of the final payload line on phone before approving. The independent failed-queue banner has been repaired in this round.
- Assistant Memory: unsaved profile navigation away has no leave guard; test explicit tab/route navigation and preserve or confirm draft loss. Save-time edits are now protected, navigation-time edits are a separate case.
- Assistant tabs: assess physical Left/Right direction in RTL (logical index always advances on Right).
- History: validate loaded conversation/resume against a daemon that implements history/page; original screenshots were a real 404. Mocked filtering checks verify UI behavior but cannot establish live transcript indexing or resume correctness. Date-window filtering may leave an invisible selected conversation; inspect the selectedKey/visible relationship.
- Usage: budget save/error and scope/window racing require a fresh pass; no claim of complete budget workflow coverage. Phone attribution intentionally hides secondary columns but CSV retains them.
- Insights: Reports/Health uses role=tablist but lacks arrow/Home/End handlers. Phone deep-link selection does not set phoneDetail; test direct report routes. Report generation/polling replacement of an existing period needs a separate check, since completion currently infers success from report-count growth.

E2E TypeScript gate passed: `npx tsc --noEmit -p tsconfig.e2e.json` (exit 0). Scoped `git diff --check` passed. No full npm check run here; parent owns the shared gate. No native Tauri window bridge or screen-reader verification.

Insights Health received source inspection only; loaded dependency expansion and support-bundle download were not exercised in this round and are not represented as verified main flows.
