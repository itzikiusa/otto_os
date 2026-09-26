# Round 3 reviewer 3/10 — Settings, MCP, Plugins and Skills

Worktree only: `/Users/itziklavon/otto-ux-audit-20260925`. No commits, subagents, real plugin installs/provider-home writes/account setup/backup restores or external MCP calls. Parent owns global checks and integration. All five rounds remain mandatory.

## Independently rechecked prior repairs

The combined run re-executed all 20 prior tests (six R1, eleven R2, three MCP default-view). Those tests passed: membership read failure blocks edits, repeated Enter mints one token, plugin list Retry, all Settings sections reachable/titled at desktop and phone, skill search/Files; all/partial role failures and remaining-only retry, overlapping file loads, edits during save with actual persisted bytes, permission rollback/retry, appearance persistence, long plugin layout/RTL paths, install failure/duplicate submission, workspace-switch MCP attachment, five-theme loaded pages, Skills error/discard/revert, MCP catalog Retry/exposure rollback. This is independent runtime evidence, not acceptance of prior reports.

The first attempted regression run was disrupted by my mistakenly overlapping another Playwright invocation on the same slot. Its last three R1 failures showed daemon startup/unavailability, not UI regressions. I waited for those processes to exit and reran sequentially. The completed combined rerun has 27 passes and one new-fixture failure described below; every prior test passed.

## Confirmed issues and repairs

1. **P2 — plugin identity only available through hover.** `ui/src/modules/settings/PluginsSettings.svelte:189,276`. At 375px, name/version/source were all ellipsized; there was no touch-accessible full value. Screenshot viewed; red test could not open any details. Added a native keyboard/touch disclosure with full name, version, source, identifier and description; paths remain LTR and wrap. Tests assert full contents and each detail's internal width, not merely page overflow.
2. **P2 — tablet skill editing remains cramped after R2.** `ui/src/modules/skills-lab/SkillsLabPage.svelte:87`, `SkillsBrowser.svelte:334`. At 834px the global sidebar plus skill list left a ~294px editor. Screenshot viewed. Added an accessible Show/Hide skills list control at desktop/tablet widths; hiding preserves the mounted editor, draft, selected skill and file. Verified editor width >480px (screenshot ~574px), draft preserved after hide/show. Existing phone push navigation stays active.
3. **P2 — an obsolete skill failure is shown under another skill's identity.** `ui/src/modules/skills-lab/SkillDetail.svelte:90-123`. Load Alpha, hold Beta's GET, select Alpha, then return Beta's 503. Red showed “Couldn't open ... Alpha. Old skill failed”. The old success-only key guard did not guard catch/finally, and returning to the last loaded key could skip a fresh load. Loads now follow a stable selected-copy key and a generation guards success, failure and loading completion. The stable key also avoids re-fetching when unrelated group/cache data changes.
4. **P2 — selecting another access group silently discards unsaved fields.** `ui/src/modules/settings/AccessGroups.svelte:106-119`, `:274,288,363,376`. Edit Alpha then select Beta: red found no confirmation and Alpha's draft was gone. Group/preset selection and New now confirm before replacing a dirty form. Internal post-save/delete initialization stays direct. Tests cover cancel preserving the group draft, discard switching group, preset cancel, and New preset discard.

No confirmed feasible issue remains deferred from this pass. No PluginFrame production change was needed.

## Deeper flows and prior remaining-workflow accounting

- **Installed PluginFrame (new scope):** safe synthetic iframe at the actual `#/plugin/synthetic-plugin` route. 503 -> Retry -> initialized frame; Reload; forwarded command-palette keyboard shortcut; 404 disabled/uninstalled/UI-less state -> Settings. Theme initialization is asserted in all five combinations. The browser host is reviewed; arbitrary third-party plugin UI quality is outside Otto's control. The same unavailable contract covers disabled, removed and UI-less plugins; these do not have separate HTTP payloads.
- **Plugin administration:** full metadata now readable on touch. Prior intercepted install/enable failure, remove-cancel and list recovery regressions rerun. No real process installed/enabled.
- **Skills:** cross-skill failure race plus tablet navigation/draft preservation added; prior per-file/load/save races rerun. Review and Evaluator list failure -> Retry -> form/empty history now checked. Running-review response races, evaluation result/promote/golden/matrix workflows and provider-copy operations remain unverified.
- **Groups & access:** group/preset unsaved navigation added. Earlier user membership/bulk writes rerun. Deep group membership writes/resource policy editing remain unverified.
- **Backups:** intercepted archive-preview/review gate, expired-preview failure, fresh preview requiring renewed review, and successful fixture response tested. No actual archive restored. Real restore I/O remains outside this review, and is not necessary to validate the UI's confirmation/retry flow.
- **MCP external:** intercepted discovery failure/retry and delete-confirm cancel added. Initially the synthetic server lacked a capabilities response, correctly leaving Discover disabled; supplied a contract-faithful legacy capabilities fixture. No production auth change. Built-in catalog/attachment/exposure regressions rerun. Policy authoring, governed tool tester and approvals remain unverified.
- **Other Settings categories:** all available category routes/titles rechecked at desktop/phone, including preferences, providers, integrations, account screens and administrative pages. This is navigation coverage, not a claim that all account/setup mutations were exercised. Provider/account setup is still unverified.

## Commands and outcomes

All Playwright commands from `worktree/ui`, common prefix:

```sh
OTTO_E2E_SLOT=ux3settings OTTO_E2E_PORT=7852 OTTO_E2E_PW_PORT=5352 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- Combined prior + R3: `npx playwright test e2e/desktop-ux-r3-settings.spec.ts e2e/desktop-ux-settings.spec.ts e2e/desktop-ux-r2-settings.spec.ts e2e/desktop-mcp-cp-default-view.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-3-settings-final`: **27 passed, 1 failed (3.5m)**. Only failure was the missing synthetic MCP capabilities fixture; all 20 prior regressions passed. Log `/tmp/otto-ux-3-settings-final.log`.
- Red reproduction after sequential restart: `npx playwright test e2e/desktop-ux-r3-settings.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-3-settings-red2`: **3 failed (expected), 1 passed**. Missing disclosure/collapse and obsolete skill failure reproduced. `/tmp/otto-ux-3-settings-pass1` then **4 passed, 1 failed**; only new group draft-loss test failed. Matching `.log` files retained.
- Strengthened final R3: `npx playwright test e2e/desktop-ux-r3-settings.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-3-settings-verified`: **9 passed (1.4m)**. Log `/tmp/otto-ux-3-settings-verified.log`.
- Actual iPhone WebKit: `npx playwright test e2e/desktop-ux-r3-settings.spec.ts --grep 'phone plugin|PluginFrame|Groups|MCP external|backup restore|Review and Evaluator' --project=iphone-portrait --workers=1 --output=/tmp/otto-ux-3-settings-webkit`: **6 passed (46.1s)**. Log `/tmp/otto-ux-3-settings-webkit.log`.
- Final desktop and WebKit `.last-run.json` both report `status: passed` and `failedTests: []`; both Playwright processes exited 0 before handoff. There are no pending tests.
- `npx tsc --noEmit -p tsconfig.e2e.json`: passed after correcting a fixture's workspace string type. `/tmp/otto-ux-3-settings-tsc.log`.
- `node scripts/ui-guards.mjs`: passed, 803 files; no baseline increase. `/tmp/otto-ux-3-settings-guards.log`.
- Scoped `git diff --check`: passed. Full npm check/build intentionally left to parent.

## Visual evidence actually viewed

Fresh screenshots use synthetic plugin/selected-skill records. Prefix `/tmp/otto-ux-r3-settings-`:

- `plugin-before.png`, `plugin-details-phone.png`: touch truncation and expanded values.
- `skills-tablet-before.png`, `skills-tablet-focused.png`: actual 834px editor before/after, with retained unsaved content.
- `plugin-frame.png`, `frame-native-light.png`, `frame-warm-light-tablet-rtl.png`, `frame-pro-dark-phone.png`: route-correct hosted iframe.
- `metadata-native-dark-phone.png`, `metadata-warm-light-tablet-rtl.png`, `metadata-warm-dark.png`: detail wrapping and Native/Warm colour schemes.
- `review-recovery.png`, `evaluator-recovery.png`: actual Review/Evaluator forms following history recovery; viewed both desktop captures and the later iPhone WebKit captures at the same paths.
- Also viewed fresh R2 rerun evidence `/tmp/otto-ux-r2-settings-skills-warm-light-tablet-rtl.png`, `skills-pro-dark-phone.png`, `mcp-native-dark-phone.png`, `appearance-native-light.png`, `appearance-pro-dark-phone.png`, `mcp-warm-light-tablet-rtl.png` to independently inspect old fixes.

Expanded metadata is intentionally scrollable on phone. The skill-list collapse leaves all editing actions readable and uses a stable button to restore navigation. Review/Evaluator forms are dense and still merit deeper loaded-result/mobile inspection. Synthetic iframe's internal HTML is a transport/host fixture, not a claimed redesign of plugin content. No real account/session/usage screenshot should be published.

## Scores (verified scope only)

| Family / inspected variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
|---|---:|---:|---:|---:|---:|
| Settings appearance/preferences/permissions/token flows | 9.5 | 9.5 | 9.3 | 9.5 | 9.4 |
| Settings Users bulk memberships | 9.3 | 9.5 | 9.2 | 9.5 | 9.2 |
| Groups/preset draft navigation | 9.3 | 9.5 | 9.3 | 9.3 | 9.2 |
| Backup restore preview/retry form | 9.3 | 9.5 | 9.3 | 9.5 | 9.3 |
| Plugin administration (including expanded phone identity) | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Installed PluginFrame host / loading/error/unavailable | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| MCP built-in server/catalog | 9.4 | 9.5 | 9.3 | 9.4 | 9.3 |
| MCP external registry/discovery | 9.3 | 9.4 | 9.3 | 9.4 | 9.2 |
| Skills Files/drafts, including focused tablet editing | 9.5 | 9.5 | 9.3 | 9.5 | 9.5 |
| Skills Review/Evaluator forms and history recovery | 9.2 | 9.2 | 9.2 | 9.4 | 9.1 |

| Visual combination | Plugin administration | PluginFrame host | Skills Files |
|---|---:|---:|---:|
| Native light desktop | 9.5 | 9.5 | 9.5 |
| Native dark phone | 9.5 | 9.5 | 9.4 |
| Warm light tablet RTL | 9.4 | 9.5 | 9.4 |
| Warm dark desktop | 9.5 | 9.5 | 9.4 |
| Pro Dark phone | 9.5 | 9.5 | 9.4 |

Deductions: dense multi-pane administration remains slower to scan on tablet; detailed keyboard/assistive navigation is less comprehensively verified than primary mouse/touch flows; Review/Evaluator loaded results remain a substantial unverified workflow. Other Settings mutation flows receive no new detailed scores from route-only coverage. No whole-scope >=9.5 claim. Native tokens here mean browser rendering, not physical Tauri or VoiceOver validation.

## Concrete next-round targets

1. Review/Evaluator loaded histories, delayed selection responses, workspace switch, review/apply status and golden/matrix/promote with synthetic contracts.
2. Groups/presets membership failure/retry and save-during-navigation; policy/access changes using intercepted writes.
3. Provider/account setup success/failure fixtures and provider-skill copy/install response handling, with all provider-home operations intercepted.
4. Governed external MCP tool tester/policies/approvals, including keyboard/RTL and loaded long histories.
5. Tablet Skills collapse across resize/RTL and full keyboard list filtering/selection. Verify source/diff compare response races separately from the fixed main-copy load.
6. Actual text contrast/readable font-size audit in all five themes/custom accents/reduced transparency; physical macOS/VoiceOver remains a distinct validation environment.
