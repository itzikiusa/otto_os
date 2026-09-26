# Reviewer 6/10 — round 1 Settings, MCP, Plugins, Skills Lab

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. No commits. Only own module files and `ui/e2e/desktop-ux-settings.spec.ts` changed.

## Confirmed defects repaired

1. **P1 — failed membership read presented as no access, with destructive edits enabled.** `ui/src/modules/settings/Users.svelte:96`, `:125`, `:565`. By-user mode previously converted each failed GET into an empty membership list, rendered every role as None, and enabled full-list replacement writes. Browser intercepted membership GETs with 503: before fix there was no error/retry and the role controls remained available. Now failed aggregate reads render an inline LoadState with Retry and block edits until all membership lists are available. Retry recovery verified against real isolated daemon.
2. **P2 — repeated Enter bypassed disabled token button.** `ui/src/modules/settings/PersonalAccessTokens.svelte:75`. The input's Enter handler directly calls mint; disabling the button does not guard it. Browser held the POST response and pressed Enter three times: received 3 POSTs (red), then 1 POST after adding the in-flight guard (green).

## Verification

`ui/e2e/desktop-ux-settings.spec.ts` contains six retained browser tests:
- MCP ArrowRight/End/Home move selection and focus together. Passed unchanged; initial source suspicion was rejected.
- Users By-user membership failure blocks role edits; Retry recovers.
- PAT repeated Enter starts only one request.
- Actual `#/settings/plugins` load error → Retry → empty state on a 375px phone.
- Every available Settings section can be filtered/opened with its matching page title at 1440px and 375px. Includes preferences, permissions, providers, backups, access groups, tokens and integrations.
- Skills Lab seeded library skill → search → selected detail → Files tab, Save correctly disabled for unchanged content.

First green run: 5 passed, 1.3 minutes. Follow-up retained Skills Lab test passed (14.4 seconds). A tentative bulk-role test passed too early and was removed; it is not counted as validation. `npx tsc --noEmit -p tsconfig.e2e.json` passed. Scoped `git diff --check` passed. Parent owns full npm check.

E2E command: `OTTO_E2E_SLOT=uxsettings OTTO_E2E_PORT=7817 OTTO_E2E_PW_PORT=5317 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/claude_ade/target/debug/ottod npx playwright test e2e/desktop-ux-settings.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-settings-results` from worktree/ui.

Red evidence retained under `/tmp/otto-ux-settings-red`; follow-up output `/tmp/otto-ux-settings-followup`. Initial token red observed expected 1 / received 3; first output directory was reused for green.

## Visual review

Actually viewed native light/dark, Warm dark, phone light and tablet RTL images across Settings Appearance, Assistant, Users, MCP and Skills Lab. Checked page/header composition, long catalog rows, form hierarchy, populated skill list/detail, theme consistency and phone stacking. Also inspected the supplied Plugins phone/dark screenshots: they show FirstRunCoach rather than Plugins. Parent owns coach repair and reported it fixed. A real phone Plugins screenshot was produced and viewed at `/tmp/otto-ux-settings-plugins-phone.png`; source input, Browse/Install controls, explanatory text and empty state fit correctly.

Visual assets are baseline route snapshots, not proof of every Settings subsection's full workflow. Settings navigation test establishes reachability/title at two widths, not full deep interaction coverage.

## Scores after repairs

| Surface | Layout | Interaction | Accessibility | States | Responsive | Mean |
|---|---:|---:|---:|---:|---:|---:|
| Settings incl. permissions/preferences | 9.2 | 8.8 | 9.0 | 8.8 | 9.0 | 8.96 |
| MCP | 9.1 | 9.1 | 9.2 | 8.9 | 9.1 | 9.08 |
| Plugins | 9.2 | 9.0 | 9.1 | 9.3 | 9.2 | 9.16 |
| Skills Lab | 9.2 | 9.0 | 8.8 | 9.0 | 8.8 | 8.96 |

Overall mean: **9.05/10**. No 9.5 claim: deep flows remain untested and the following round-two checks remain.

## Residuals / fresh reviewer handoff

- **Shared RTL code styling:** `/tmp/otto-ux-screenshots/tablet-rtl-skills-eval.png` visibly reorders code punctuation (`./q` command). `SkillDetail.svelte:405` renders `.md-body`; shared `ui/src/app.css:447` pre styling lacks LTR isolation. Sent to parent for reviewer10; no shared CSS changed.
- **Bulk role results:** `Users.svelte:147-156` shows success unconditionally after calls to `setRoleIn`, which catches request failures internally (`:139`). Source indicates misleading success after partial/all failure; reproduce with delayed PUT failures and wait for all rows to finish before asserting. Tentative too-early browser test was removed to avoid false coverage.
- Skills editor overlapping-file loads/save-during-edit and long plugin version/name layout were source-inspected but not fully stress-tested. No speculative changes made.
- No actual plugin install/enable or outward MCP tool mutation was performed. The review exercised read/navigation, isolated fixture seeding and intercepted failures.
