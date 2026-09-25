# Round 4 reviewer 1/10 — Settings, MCP, Plugins, Skills Lab

Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. No commits, subagents, host provider-home writes, plugin installs, real account/cloud mutations, or backup restores. All account, fixer, promotion, MCP invocation/policy/approval/resource-policy writes used intercepted synthetic contracts. Parent owns integration and global checks.

## Independent prior verification

All **29** earlier regressions passed in a fresh combined R1/R2/R3/MCP-default run before edits (`/tmp/otto-ux-4-settings-prior.log`, 1.9m). This includes Users read failure/bulk outcomes, PAT duplicate Enter, all Settings routes, appearance persistence/permission rollback, Plugin install/enable failures and readable touch metadata, installed PluginFrame retry/theme/keyboard/unavailable states, Skills selection/file/save races and tablet draft preservation, group/preset unsaved selection, safe backup-preview recovery, built-in MCP attachment workspace race, discovery/catalog/exposure recovery, and five-theme loaded variants. Fresh rerun screenshots were actually viewed, not accepted from the old reports.

## Confirmed repairs

1. **P2 — delayed Review selection overwrites the newer choice.** `ui/src/modules/skills-lab/SkillReviewPanel.svelte:120`. Open Alpha, hold Beta GET, reselect Alpha, release Beta: red received Beta instead of Alpha. Added selection generation/workspace guards and invalidation on New, handoff, workspace change and destroy. Final regression also opens New and types instructions while Beta is delayed, then proves those instructions survive completion.
2. **P2 — loaded evaluation actions overflow on phone.** `ui/src/modules/skills-eval/RunDetail.svelte:697,857,890`. New populated fixture's rating/export/improvement rows exceeded their own widths; the tablet screenshot also showed a skill chip escaping the card. Rating/action rows now wrap, the full skill identity gets a separate wrapping line, chips stay bounded, and the report title wraps. Tests assert internal row width, scroll to the actual lower actions, and retain promotion functionality.
3. **P2 — Review and Evaluator tablet composition remains crowded.** `ui/src/modules/skills-lab/SkillReviewPanel.svelte:301,549,630`; `ui/src/modules/skills-eval/SkillsEvalPage.svelte:268,283,403`. At 834px RTL, history plus global sidebar left ~314px for loaded reports. Review finding metadata squeezed the explanation to a tiny column. Added named history-collapse controls with expanded/control state; selected report remains mounted, keyboard Enter restores/collapses the list without losing selection. Finding severity/code now precede a full-width explanation. Before/after screenshots show the report using the available ~614px. Eval toggle only appears on Runs, where it controls an actual list.
4. **P2 — phone Jira account identity squeezed by inline actions.** `ui/src/modules/settings/IssueAccounts.svelte:355,425`. Safe setup fixture rendered only **122.5px** for email/URL/connection result at375px, splitting normal identity text across many lines. Red width assertion confirmed the visual issue. A480px container query moves Test/Edit/Delete below identity; name can wrap. Final WebKit screenshot shows complete normal email/URL lines and a distinct actions row; identity width exceeds200px.

No shared component/store changes were needed. Parent offered ResourceAccess ownership if a confirmed issue arose; the exercised policy flow passed without changes.

5. **P1 — Users freezes with no workspace.** `ui/src/modules/settings/Users.svelte:265`. With no workspaces, switch to By user: `loadAllMembers()` resolves `Promise.all([])` and writes a new empty map; the effect observes that map and loops again indefinitely. Browser click hangs and cannot complete its screenshot. The effect now requires at least one workspace. Explicit empty-workspace regression verifies the role controls and subsequent navigation stay responsive; existing membership-failure/Retry regression passes after this repair.
6. **P2 — Groups detail is only118px wide on tablet.** `ui/src/modules/settings/AccessGroups.svelte:443,642`. Eight synthetic groups at834px beside global+Settings rails left118px for fields, a clipped group name and membership copy stacked one/two words per line. Replaced the existing640px viewport stacking rule with a section container query. The list now occupies a bounded180px scroll area above a full-width detail; retained regression requires detail>280px. Both Chromium and WebKit pass. Before/after images were viewed.

## New executed workflows

The ten retained tests in `ui/e2e/desktop-ux-r4-settings.spec.ts` cover:

- Review selection race and New-form retention against an old response.
- Explicit zero-workspace Users responsiveness and a loaded eight-group/six-user tablet administration fixture; usable group-detail width beside both navigation rails.
- Loaded evaluation result in Native light, Native dark phone, Warm light tablet RTL, Warm dark, Pro Dark phone; actual internal action bounds and keyboard list collapse.
- Promote improved version: proof gate display, invalid name disables submission, failed write retains name, successful retry posts the selected iteration/version/name.
- Golden task edit/save, golden task run→report, loaded matrix scored cell→the matching report.
- Mock Jira account add failure/retry and connection failure/retry; phone composition.
- MCP tool tester malformed JSON blocks requests; valid request shows pending approval; policy unknown-key validation and create; approval failure preserves note and retry posts it. Also group preset copied into resource access rule→review comparison→save, all mocked.
- Access-group save locks list selection and fields until response, then permits another selection. The hypothesized save-selection race was **rejected** because disabled controls make that path unreachable; no production change.
- Loaded static Review report in all five themes, actual composited contrast >=4.5 for heading/notes/verdict/severity and font size>=11px, internal widths, keyboard history toggle.
- Apply-fixes failure retains instructions; successful retry shows a synthetic completed fixer. No real agent spawned or provider file touched.

Account fixture originally had an ambiguous `role=status` assertion because both connection result and toast matched; scoped to `.test-result`. Resource access save intentionally invalidates/remounts the MCP subtree, closing the editor; test now asserts the success toast and saved fixture rule rather than expecting the previous editor to survive. WebKit Apply retry initially hit a persistent error toast over the button; the realistic retry flow now dismisses that named toast first. These were test/interaction assumptions, not silently weakened behavioral assertions.

## Completed verification

Common command environment, from worktree/ui:

```sh
OTTO_E2E_SLOT=ux4settings OTTO_E2E_PORT=7861 OTTO_E2E_PW_PORT=5361 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod
```

- Prior recheck: `npx playwright test e2e/desktop-ux-settings.spec.ts e2e/desktop-ux-r2-settings.spec.ts e2e/desktop-ux-r3-settings.spec.ts e2e/desktop-mcp-cp-default-view.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-4-settings-prior`: **29 passed (1.9m)**.
- New desktop workflows before final account composition change: `npx playwright test e2e/desktop-ux-r4-settings.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-4-settings-final`: **9 passed (35.7s)**; matching `.log`.
- Final WebKit: `npx playwright test e2e/desktop-ux-r4-settings.spec.ts --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-4-settings-webkit-final`: **9 passed (1.5m)**; `.last-run.json` passed/no failed tests. This is actual WebKit, with phone flows plus deliberately resized theme/tablet variants.
- Final combined desktop: `npx playwright test e2e/desktop-ux-settings.spec.ts e2e/desktop-ux-r2-settings.spec.ts e2e/desktop-ux-r3-settings.spec.ts e2e/desktop-ux-r4-settings.spec.ts e2e/desktop-mcp-cp-default-view.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-4-settings-combined`: **37 passed,1 failed (3.7m)**. Only failure: backup `page.goto net::ERR_CONNECTION_REFUSED localhost:5361`321ms. Parent/content reviewer confirmed a concurrent Vite-config edit at the matching22:12UTC time restarted dev servers. No Settings assertion failed. Focused backup rerun in `/tmp/otto-ux-4-settings-admin-red` **passed**; that run's other new test exposed the separate Users freeze.
- Final added admin regression + pending-save flow: `npx playwright test e2e/desktop-ux-r4-settings.spec.ts --grep 'tablet access administration|saving an access group' --project=desktop-browser --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-4-settings-admin-final`: **4 passed (14.1s)**; passed/no failed tests in `.last-run.json`.
- Followup after Users repair: `npx playwright test e2e/desktop-ux-settings.spec.ts e2e/desktop-ux-r2-settings.spec.ts e2e/desktop-ux-r4-settings.spec.ts --grep 'By-user membership|bulk workspace|MCP tester' --project=desktop-browser --workers=1 --output=/tmp/otto-ux-4-settings-followup`: **2 passed (11.1s)**. The grep did not match the R2 bulk test (its title is `bulk role failures`); that test passed in prior/combined runs. These2 are the R1 membership failure/Retry and R4 MCP workflow.
- Final phone MCP capture run: `npx playwright test e2e/desktop-ux-r4-settings.spec.ts --grep 'MCP tester' --project=desktop-browser --workers=1 --output=/tmp/otto-ux-4-settings-mcp-phone`: **1 passed (9.7s)**, phone375px from initial navigation; final actual loaded tester screenshot viewed.
- `npx tsc --noEmit -p tsconfig.e2e.json`: passed; `/tmp/otto-ux-4-settings-tsc.log`.
- `node scripts/ui-guards.mjs`: passed after replacing the new generic `.btn` layout selector with local `.acct-test`; `/tmp/otto-ux-4-settings-guards.log`. No baseline increase.
- Scoped `git diff --check`: passed. No full npm check/build by this reviewer.

Additional admin red evidence: `/tmp/otto-ux-4-settings-admin-red` (Users zero-workspace freeze), `/tmp/otto-ux-4-settings-admin-pass1` (group fieldset118px).

Red evidence: `/tmp/otto-ux-4-settings-red` (Review race + phone row overflow), `pass1` (tablet collapse absent), `pass4` (Review collapse absent), `/tmp/otto-ux-4-settings-account-red` (122.5px identity), matching log files. `pass2/3/5` include the documented fixture/selector corrections; final completed checks supersede them.

## Runtime caveat

The first WebKit run logged one generic `Unhandled rejection TypeError: Load failed` while reloading loaded Review themes (22:06:33UTC), without a useful stack. It was not reproduced as that error in the full repeated WebKit run with page-error stack logging. Do not call it harmless or claim it fixed. Parent was notified. The repeat instead captured a **different** HMR error during concurrent edits: `module.default` undefined at `src/modules/swarm/SwarmPage.svelte:2371:33`, through `@vite/client queueUpdate` (22:09:04UTC); reported to parent/owner. This is not attributed to Settings. Test assertions passed, but that run is not claimed console-clean. The later combined desktop run and targeted admin/followup runs logged no page-error or unhandled-rejection messages; parent will run a stable-source runtime check after all reviewer edits settle.

## Actual visual inspection

All images are synthetic fixture content, with isolated E2E account/workspace names. Prefix `/tmp/otto-ux-r4-settings-`:

- `eval-native-light.png`, `eval-native-dark-phone.png`, `eval-warm-light-tablet-rtl-before.png`, `eval-warm-light-tablet-rtl.png`, `eval-warm-dark.png`, `eval-pro-dark-phone.png`.
- `review-native-light.png`, `review-native-dark-phone-before.png`, `review-native-dark-phone.png`, `review-warm-light-tablet-rtl-before.png`, `review-warm-light-tablet-rtl.png`, `review-warm-dark.png`, `review-pro-dark-phone.png`.
- `account-phone.png` before and after card repair (after first Chromium capture, subsequently WebKit); `golden.png`, `matrix.png`, `mcp-tester.png`.
- Fresh prior rerun evidence actually viewed: `/tmp/otto-ux-r3-settings-skills-tablet-focused.png`, `plugin-details-phone.png`, `frame-warm-light-tablet-rtl.png`, `metadata-warm-dark.png`; `/tmp/otto-ux-r2-settings-mcp-native-dark-phone.png`, `appearance-native-light.png`.
- `/tmp/otto-ux-r4-settings-users-tablet.png` (initial missing grant fixture and final loaded grants), `groups-tablet-before.png` (118px), `groups-tablet.png` (stacked full-width detail), and `mcp-tester-phone.png`.
- WebKit initial Apply failure screenshot viewed to identify the error-toast interception.

Layout now gives loaded report text priority, keeps all export/improvement actions available on phone, and separates account identity from actions. Native theme means browser token rendering, not physical macOS/Tauri validation.

## Scores — inspected page families (L/I/A/S/R)

These judge verified UI quality, with explicit coverage limits; they do not assume production writes are needed to establish a UI score. Prior surfaces with no new deeper evidence retain their earlier deductions.

| Page / important variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
|---|---:|---:|---:|---:|---:|
| Settings appearance/preferences/permissions/tokens | 9.5 | 9.5 | 9.3 | 9.5 | 9.4 |
| Users bulk memberships + zero-workspace view | 9.4 | 9.5 | 9.3 | 9.5 | 9.4 |
| Groups/preset navigation, pending save, repaired tablet | 9.5 | 9.5 | 9.3 | 9.5 | 9.5 |
| Jira account setup/test/recovery, repaired phone card | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Backup preview/retry | 9.3 | 9.5 | 9.3 | 9.5 | 9.3 |
| Plugin administration including touch metadata | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Installed PluginFrame host | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| MCP built-in catalog | 9.4 | 9.5 | 9.3 | 9.4 | 9.3 |
| MCP external tester/policy/approval/group-resource access | 9.4 | 9.5 | 9.3 | 9.5 | 9.3 |
| Skills Files/drafts | 9.5 | 9.5 | 9.3 | 9.5 | 9.5 |
| Skills loaded Review/static findings/fixer form | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Skills loaded Evaluation/promotion | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Golden tasks + Matrix loaded navigation | 9.4 | 9.5 | 9.3 | 9.4 | 9.4 |

Concrete next-round composition pointers: `users-tablet.png` / `Users.svelte:446-620` shows the accounts list preceding both administration sections; even six accounts place Feature grants near the bottom of the1112px viewport. Consider a bounded/collapsible account list or direct section navigation for larger user sets, and verify tab order through the two separately selected role/grant users. This is scanning friction, not a clipped control. The former Groups118px defect is repaired and its responsive score changes only because of that measured repair. MCP phone evidence and `ToolsTab.svelte:196-331` put server metadata, tool governance switches and tester in one long sequence; verify retaining tester arguments/results across breakpoint transitions (the first desktop→phone evidence capture remounted/closed the tool disclosure, so final evidence loads phone initially). This transition is a candidate requiring direct state assertions, not a claimed data-loss bug. Golden task phone titles (`golden.png`, `GoldenTasksView.svelte:303-324`) truncate next to Run/Edit/Delete; Edit exposes the full editable name but a touch disclosure/wrapping identity would reduce that extra navigation; full scorecard/proof and active-agent report variants are not established by these completed fixtures. The reviewed static Review flow now supports readable full-width findings, maintained selection, keyboard collapse and measured five-theme text contrast, justifying that narrowly scoped9.5 row. No whole-scope>=9.5 claim.

## Remaining next-round depth

- Skill evaluation multi-signal Scorecard/proof artifacts, human rating/regression capture, compare mode, matrix creation/cancel and active validator streaming. This round established loaded iteration results, promotion, golden editing/running and matrix→report, not every evaluation capability.
- Review multi-agent summary/terminal streaming, pending start/apply responses across selection/workspace, and initial history response arriving after a form draft. These remain candidates to reproduce, not asserted bugs.
- Broader provider setup/copy/install branches remain untested; all host homes must stay protected. Jira setup is now specifically established.
- Full group membership/resource-policy failure matrix, nested delegation/children/effective-access views. Basic group preset→preview→save is now executed.
- Golden-task long identity readability without entering Edit and larger matrix grids merit composition review; custom accents, physical VoiceOver/Tauri and all theme×viewport combinations outside the explicitly tested matrices remain separate verification.
- Trace the isolated generic WebKit Load failed if it recurs in a stable, non-HMR environment. Do not conflate it with the later stack-attributed Swarm HMR error.

All listed final processes have exited; no test is pending at handoff. Production edits are limited to six owned components: IssueAccounts, Users, AccessGroups, SkillsEvalPage, RunDetail, SkillReviewPanel, plus the new R4 spec. No shared files were edited.

## Parent integration check

The parent independently reproduced an additional active-review race: while Alpha was running, selecting a delayed Beta allowed Alpha's fallback poll to supersede that explicit request. The new regression failed with Alpha still displayed after Beta completed (`/tmp/otto-ux-r4-parent-review-red.log`). Background refreshes now defer to a pending selection and coalesce within its generation; they cannot invalidate an explicit choice. The retained R4 spec now contains11 tests.

Parent verification of the new polling case, previous late-selection case, account setup/retry, Apply-fixes retry and tablet Users/Groups behavior passed **10/10 across Chromium and WebKit**, 37.8s, exit0 (`/tmp/otto-ux-r4-parent-settings.log`, `/tmp/otto-ux-r4-parent-settings-results`). Full integrated `npm run check` also passed with zero errors and zero warnings (`/tmp/otto-ux-r4-integration-check.log`). Scores remain the reviewer's per-surface judgments above; this check does not certify untested variants.
