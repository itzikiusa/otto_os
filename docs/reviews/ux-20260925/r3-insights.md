# Round 3 — reviewer 9/10: Assistant, Memory, History, Usage, Insights

Worktree only `/Users/itziklavon/otto-ux-audit-20260925`. No commits/subagents or original-checkout edits. All outward Assistant actions, attachments, provider pinning, report runs, budgets and support bundles used synthetic contract fixtures. Actual History read/resume used the existing isolated temporary transcript + fake provider CLI regression. Screenshot contents are synthetic. Parent approved the additive Insights Rust/API/type change; no shared component edits.

## Prior repairs independently rechecked

Initial independent run of `desktop-ux-r2-insights.spec.ts desktop-ux-insights.spec.ts desktop-insights.spec.ts`: **26 passed**, including full phone approval payload review before Send, Memory save-time draft preservation and leave guard, RTL physical tabs, filtered History selection, Usage budget save snapshot/error/window races, regenerated same-period report detection, Insights deep links, Health keyboard/Retry/dependencies/download, loaded synthetic History transcript/import retry/resume, and real isolated report artifacts/Markdown/sandboxed HTML.

Five-theme Memory/Reports screenshots were regenerated, then actually inspected (paths below). Prior repair assertions were retained. The one R2 fixture change adds the new `report_key` response for its fixed synthetic September 24 report, so the fixture follows the new contract instead of depending on the host date. It does not weaken any assertion.

## Confirmed findings and repairs

| Severity | Surface | Reachable reproduction and repair |
|---|---|---|
| P1 | `ui/src/modules/insights/InsightsPage.svelte:411,454`; `crates/otto-server/src/insights.rs:211,652` | Manual daily run pending, scheduled weekly report arrives: the UI announced the unrelated report ready and stopped polling. Backend reachability: scheduler `tick` independently spawns each cadence, while manual `post_run` calls the runner without that scheduler's in-flight map. Added optional `report_key` to RunInsightsResp, derived from daemon-local calendar and requested kind/offset; polling compares that key against the pre-request snapshot. Older daemons use a browser-local compatibility fallback. Tests show unrelated report arrival keeps Running disabled and the actual daily replacement completes. |
| P2 | `ui/src/modules/insights/InsightsPage.svelte:222` | A delayed old full-summary response overwrote the regenerated report body. Effect cleanup now invalidates its response when the selected report/version changes or the view unmounts. Regression deliberately releases old Markdown after the new body appears. |
| P2 | `ui/src/modules/insights/InsightsPage.svelte:138` | Same race in index.json: current 88-session metrics reverted to 12 after an old index response. Latest-request/disposal checks now protect the index. Separate regression failed before repair, passed after. |
| P1 | `ui/src/modules/assistant/AssistantComposer.svelte:62` | Submit a message, type another draft while POST waits, release 503: the old message replaced the newer draft. Recovery preserves both texts (separated by a blank line) and merges attachments. Regression asserts both messages remain. |
| P2 | `ui/src/modules/assistant/AssistantComposer.svelte:33` | Drafts and uploads were component-local and discarded by thread/tab navigation. Shared unsaved-change guard now covers text, attachments and pending send/upload. Keep editing retains the composer. Explicit discard remains available. |
| P2 | `ui/src/modules/assistant/AssistantComposer.svelte:80,277` | Phone attachment name was ellipsized with full identity only in a hover title; after a failed attempt, successful retry retained the stale error. Files now wrap, keep size/remove visible, and a new upload attempt clears the prior error. Regression verifies full measured filename, upload failure/retry/removal, then model-pin failure/retry. |
| P2 | `ui/src/modules/usage/UsagePage.svelte:56,57,939` | Negative budget window was saveable despite the HTML min; navigation discarded an unsaved budget draft. Added integer 1–3650 validation, nonnegative finite cap validation, blur-triggered inline error/ARIA, disabled invalid Save, and unsaved-change guard. Server bounds remain unchanged. Scope regression adds/removes provider/workspace rows, saves enforce+block settings, and checks exact submitted scopes/caps. |
| P2 | `ui/src/modules/agents/history/HistoryPage.svelte:951` | A selected long History title remained truncated on phone, visible only through hover. Detail heading now wraps. Test also releases an older Claude pagination response after changing provider to Codex; stale rows remain excluded. |

The existing keyed ChatView/store ticket handling passed the delayed thread-load test: late personal-thread turns did not replace the selected Work thread. Memory's explicit Discard branch also passed. No source changes were needed for either.

## Exact commands and results

All Playwright runs used cwd `/Users/itziklavon/otto-ux-audit-20260925/ui` and this prefix:

```sh
OTTO_E2E_SLOT=ux3insights OTTO_E2E_PORT=7858 OTTO_E2E_PW_PORT=5358 \
OTTO_E2E_SWEEP_ORPHANS=0 \
OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod \
npx playwright test <arguments below> --workers=1 --output=<output>
```

Only one Playwright process ran in this slot at a time; each exited before the next started.

- Prior: `desktop-ux-r2-insights.spec.ts desktop-ux-insights.spec.ts desktop-insights.spec.ts --project=desktop-browser`; output `/tmp/otto-ux-r3-insights-prior`, log same stem `.log`: **26 passed (1.6m), exit 0**.
- Red: `desktop-ux-r3-insights.spec.ts --project=desktop-browser`; output/log stem `/tmp/otto-ux-r3-insights-red`: **3 expected failures** (unrelated report accepted, newer message overwritten, invalid budget Save enabled).
- Second: same arguments, stem `/tmp/otto-ux-r3-insights-second`: **3 passed, 1 failed**, delayed old Markdown reproduced. The report correlation edit landed while this intermediate run was active; stable later runs are the evidence for its success.
- Third: same arguments, stem `/tmp/otto-ux-r3-insights-third`: **4 passed, 2 fixture failures**. Removed a premature `isVisible` branch on the phone direct URL and matched the actual `turns?limit=200` GET route. These were fixture issues, not product findings.
- Depth red: `desktop-ux-r3-insights.spec.ts --grep 'attachment|Memory discard|History provider' --project=desktop-browser`; stem `/tmp/otto-ux-r3-insights-depth-red`: **1 passed, 2 expected clipping failures** (filename and History title).
- Combined: `desktop-ux-r3-insights.spec.ts desktop-ux-r2-insights.spec.ts desktop-ux-insights.spec.ts desktop-insights.spec.ts desktop-assistant.spec.ts --project=desktop-browser`; stem `/tmp/otto-ux-r3-insights-final`: **47 passed (2.7m), exit 0**. Existing Assistant suite includes approval, memory Undo/review, browser handoff, model routing, loaded/empty/error, phone push navigation, axe and serious contrast checks.
- Index red: `desktop-ux-r3-insights.spec.ts --grep 'old index|scope caps' --project=desktop-browser`; stem `/tmp/otto-ux-r3-insights-index-red`: **1 expected index race failure, 1 passed budget scope workflow**.
- **Final R3 suite:** `desktop-ux-r3-insights.spec.ts --project=desktop-browser`; stem `/tmp/otto-ux-r3-insights-green`: **9 passed (46.3s), exit 0**, `.last-run.json` passed/empty failedTests. This used the rebuilt daemon with report_key backend.
- **WebKit phone:** `desktop-ux-r3-insights.spec.ts desktop-ux-r2-insights.spec.ts --grep 'phone approval|phone report deep link|Health keyboard|attachment recovery' --project=iphone-portrait`; stem `/tmp/otto-ux-r3-insights-webkit`: **4 passed (24.9s), exit 0**. Same port/slot, after the desktop run ended.
- `npx tsc --noEmit -p tsconfig.e2e.json`: latest `/tmp/otto-ux-r3-insights-tsc-green.log`, **exit 0**. One intermediate type check failed because importing a fixture type from a rune store pulled `$state` into the non-Svelte E2E tsconfig; replaced it with the small fixture wire shape. No production tsconfig suppression/change.
- `node scripts/ui-guards.mjs`: initial `/tmp/otto-ux-r3-insights-guards.log` **exit 0**. Final aggregate rerun `/tmp/otto-ux-r3-insights-guards-final.log` **failed** on the concurrently edited Help `FirstRunCoach.svelte` global-class count (6 vs baseline 5); no Insights-owned file was flagged. Parent notified Help owner and owns the rerun. No baseline edits.
- Scoped `git diff --check` across all listed changed files: **exit 0**.
- Parent ran `cargo test -p otto-server insights::tests`: `/tmp/otto-ux-r3-insights-rust.log`, **12 passed / 0 failed**, including new calendar/offset test (year boundary, ISO week, leap month, invalid offsets). Parent rebuilt debug daemon successfully: `/tmp/otto-ux-r3-insights-daemon.log`. I read these outputs. Parent owns global Rust/UI/build gates.

Observed console warnings: intentional service-worker blocking; one transient `uiCommands` missing-handlers warning during concurrent shared-file development/HMR in the combined run. No new shared-command change made here. Final scoped tests passed against stable scoped sources.

## Rendered evidence actually viewed

Under `/tmp/otto-ux-screenshots/`:

- `insights-r3-chat-recovery.png` — desktop Native light, 1280×800, both retained draft messages and inline failure.
- `insights-r3-attachment-phone.png` — 390×844, Native light, complete filename/size/remove and composer; inspected Chromium version and later WebKit overwrite.
- `insights-r3-history-phone.png` — 390×844 Native light, full multiline selected title, resume/open/copy controls.
- `insights-r3-usage-rtl-tablet.png` — 834×1112 Warm dark RTL, saved workspace/provider caps and enforcement controls.
- Regenerated `insights-r2-approval-phone.png`, `insights-r2-health-phone.png` — phone complete-payload disclosure and Health recovery/dependency/bundle. Viewed again after WebKit regenerated them.
- `insights-r2-history-real-synthetic.png` — actual backend-read two-turn synthetic conversation, 1280×800.
- `insights-r2-usage.png` — 1280×800 loaded synthetic provider/7-day All summary after stale error race.
- `insights-r2-memory-native-light-1440.png`, `insights-r2-memory-warm-light-390.png`, `insights-r2-memory-warm-dark-1024.png`, `insights-r2-memory-pro-dark-dark-1440.png` — loaded profile/review/memories at desktop, phone and RTL tablet.
- `insights-r2-reports-native-light-1440.png`, `insights-r2-reports-native-dark-1440.png`, `insights-r2-reports-warm-light-390.png`, `insights-r2-reports-warm-dark-1024.png`, `insights-r2-reports-pro-dark-dark-1440.png` — all five theme/scheme variants, rich metrics/actions/summary and phone/RTL.
- `insights-r1-tasks-dark-loaded.png` — regenerated Native dark Tasks, approvals and running tasks.

This is a theme/viewport matrix across the reviewed family, not a claim that every individual page was visually checked in every theme.

## Per-page / important-variant scores

L/I/A/S/R = layout-readability / interaction / accessibility / states-recovery / responsiveness. Scores assess the observed UI, not whether a production account was mutated.

| Page/variant | L | I | A | S | R | Mean |
|---|---:|---:|---:|---:|---:|---:|
| Assistant Chat, desktop send recovery + phone attachment/model | 9.5 | 9.5 | 9.4 | 9.6 | 9.5 | 9.50 |
| Assistant Tasks, Native dark + phone complete approval | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 | 9.52 |
| Assistant Memory, loaded desktop/phone/RTL + both leave branches | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 | 9.52 |
| History, loaded real synthetic transcript + filtered/paginated phone | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 | 9.48 |
| Usage, scope/window/save races + RTL tablet cap editor | 9.5 | 9.5 | 9.5 | 9.6 | 9.5 | 9.52 |
| Insights Reports, all five themes + concurrency/refresh | 9.6 | 9.6 | 9.5 | 9.6 | 9.5 | 9.56 |
| Insights Health, phone keyboard/error/retry/bundle | 9.5 | 9.5 | 9.5 | 9.6 | 9.5 | 9.52 |

History stays below 9.5: full keyboard traversal and very long multi-page transcript reading remain incompletely exercised. This round substantially verified provider/page race and touch identity, but does not claim all long-history interactions are established. Chat's accessibility deduction reflects incomplete focused checks of streaming announcements and attachment-only draft recovery; its tested interaction/recovery improved. No score increase comes merely from the target or round number.

## Explicit remaining gaps for rounds 4/5

- Assistant actual incremental transcript streaming/cancellation, attachment-only failed-send recovery, multi-file partial-upload errors, and drafts across an explicit workspace/logout transition need deeper coverage. Current pending thread GET and send errors are covered.
- History very long transcript pagination + keyboard reading/filter/deep-link interactions, workspace switch during a pending first-page load, and real on-disk import mutation remain gaps. Import response was mocked, then isolated fake-CLI resume exercised.
- Usage cap enforcement effects on a newly started run are backend policy outside this UI scope; duplicate scope editing and auto-refresh during a pending budget PUT remain follow-ups. New tests assert the exact scope payload and preservation, not real account billing.
- Insights report_key uses server-local calendar at request acceptance; the collector independently evaluates `datetime.now()` when it starts. A delayed start across midnight/week/month rollover could target another period. This is a traced boundary risk, not a reproduced UX finding in this round. Forcing `--start/--end` would change collector kind to adhoc, so no speculative skill/protocol expansion was made. The older-daemon compatibility fallback uses the browser timezone. Generation timeout recovery and start-during-initial-load deserve direct tests.
- Health support-bundle failure/retry and more varied dependency lists still need expansion.
- No native Tauri bridge, physical device/VoiceOver, or full custom-accent contrast matrix was exercised. Four selected flows did run in WebKit iPhone emulation. Existing serious contrast checks passed; five-theme visual inspection is not a computed contrast claim.
- All five rounds remain mandatory; this is only round 3 of the assigned page family.
