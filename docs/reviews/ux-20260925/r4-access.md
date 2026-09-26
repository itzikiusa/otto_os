# Round 4 reviewer — shared access and theme accessibility

Worktree `/Users/itziklavon/otto-ux-audit-20260925` only. No commits, subagents, real account/provider writes, or full npm check/build. Parent owns integration. Shell owner explicitly granted the small auth-identity reload effect in `ui/src/shell/App.svelte`; its layout/Drawer changes are not mine. R3 FirstRunCoach bootstrap repair was preserved.

## Confirmed findings and repairs

1. **P2 — Skip retried the unwanted optional workspace.** `ui/src/modules/settings/Onboarding.svelte:79,170`. Root POST succeeds, workspace POST fails; Back twice → Skip → Finish retried the same populated fields and trapped the user. Store the explicit Skip/Continue choice independently from the draft fields. The test asserts successful shell entry, one root POST and exactly one failed workspace POST.
2. **P2 — lost root response left first-account setup stuck.** `Onboarding.svelte:62`. An aborted response after account creation followed by server `409 already onboarded` never recovered. For ambiguous transport/server failures or conflict, authenticate root using the password the user just entered; preserve the original error if that recovery fails. Do not create/replace another account. The contract-faithful test intercepts account/login/workspace writes and checks the exact login body and successful shell entry.
3. **P2 — wizard keyboard focus and validation context were lost between steps.** `Onboarding.svelte:11,134,148`. Enter on Get Started removed the focused button without moving focus to the new step. Step headings now receive programmatic focus, progress exposes its step count, and confirmation-password mismatch has associated live validation and `aria-invalid`. Back is disabled during the irreversible finishing request. R3 optional workspace failure/retry and long tool-row repairs still pass.
4. **P2 — RTL reversed the visible workspace path.** `Onboarding.svelte:166`. Fresh Warm-light and Pro-Dark screenshots displayed `tmp/synthetic/review/` despite input `/tmp/synthetic/review`. Computed direction regression failed with `rtl`; the directory input now has `dir=ltr`.
5. **P2 — closing a child sheet could leave focus on the document after its trigger disappeared or became inert.** `ui/src/lib/dialogFocus.ts:31`, `ui/src/lib/components/Modal.svelte:71`. Actual New session → Browse → picker, followed by a synthetic parent-control availability update, failed both removed/inert cases. Capture the containing dialog as a fallback scope and focus its first usable control when the original target cannot accept focus. Filter inert controls from traps. Old pointer/keyboard/async-import focus regressions are retained.
6. **P2 — delayed intrinsic media sizing did not expose an overflowing sheet to keyboard users.** `Modal.svelte:22`. A height-capped real confirmation body acquired a 900px SVG after load while the body's own height stayed fixed; `scrollHeight` grew but tabindex remained absent. Observe immediate content children as well as the body, refreshing observations when content changes. This is an isolated shared-primitive fixture (an embedded preview viewport), not a claim about a specific production attachment workflow.
7. **P2 — old workspace responses replaced the new effective identity.** `ui/src/lib/stores/workspace.svelte.ts:438`. Hold an old list GET, load the replacement token's list, release the old response: both current selection and list returned to `old`. Add request-generation, token, and selection ownership; clear old identity data before a new identity load. Use the captured result list after awaits, eliminating the earlier reactive-after-await list read.
8. **P2 — impersonation did not reload workspace scope.** `ui/src/shell/App.svelte:300`. Actual `auth.impersonate` changed the identity but the shell kept the old workspace list. A separate effect tracks `auth.me.id` and invokes `ws.load` untracked. Event/native subscriptions remain once per window. The new browser test drives the actual auth method, not a direct assignment to its user fields.
9. **P2 — old cross-workspace session results repopulated the replacement identity.** `workspace.svelte.ts:287`. Hold `/workspaces/other/sessions`, impersonate, wait for new selection, release old result: `otherWsSessions` acquired `old-private`. Guard the background results by captured token and selection generation. This is stale UI disclosure of already-fetched data, not a claim that backend access control accepted a new unauthorized request.
10. **P2 — custom accents made links and input focus disappear.** `ui/src/lib/accent.ts:44`, `ui/src/lib/stores/ui.svelte.ts:836`, `ui/src/app.css:93,231`. With bright yellow light / near-black dark accents, real rendered Markdown links measured **2.07–2.51:1** and focused input boundaries **1.06–1.60:1** across all five themes. Compute the strongest retained custom hue that clears the active theme's solid surfaces and accent-tinted selections; publish it as `--accent-text`. Global focus outlines and input borders use this readable token; primary fills retain their existing contrast-safe pair. Wizard progress indicators use the same readable tint. Updated the design guidelines' focus-token description.

## Independent prior verification and fixture honesty

- First independent R1/R2/R3 run: **75 passed, 1 failed**. R1 Pro-Dark Skills clicked the fixture row but its preview briefly contained the default SQL skill (two pre blocks). The assertion was not weakened. Isolated unchanged rerun passed, and the full combined final rerun passed every prior test. Parent received the initial screenshot/trace; this remains an intermittent Skills selection observation for R5, not a claimed access-source repair.
- Initial R4 run: six failures. Five were the setup/focus/intrinsic-size product failures above. The workspace fixture initially omitted `/workspaces/:id/sessions`, so that failure was unauthorized before the intended assertion; corrected that transport, then reproduced actual new→old overwrite. The fixture's `/auth/me` also stays synthetic so auth verification cannot accidentally log it out.
- Extended initial run: workspace ownership and heading focus failed as intended. All setup contrast/axe assertions already passed; the five later confirmation checks failed because ConfirmDialog is mounted only in the authenticated shell. Corrected the fixture to finish safely mocked setup before requesting a confirmation; no production dialog mount change.
- Actual impersonation-only regression failed before the shell effect. Cross-workspace-session and RTL-path regressions both failed before repairs.
- Custom-accent regression failed in all five themes before repair, recording both link and focus contrast failures. A combined run later had two WebKit **test helper** errors because an in-progress CSS transition serialized to `oklab`, not rgb/srgb. Normalize supported computed colors using a 1px sRGB canvas and wait for the final border color. No contrast threshold was lowered, and no production animation was disabled to hide failure.
- Late scratch discovery/new workspace selection is explicitly exercised. It is a protected invariant/negative candidate, not an additional asserted baseline bug.
- Synthetic R3 login/impersonation tests retain known unauthorized WorkspaceStore logs when their fake bearer reaches a real isolated list endpoint; new scope-transition tests intercept those reads and assert real identity/selection state. The old fixture warnings are not production auth evidence.

## Commands and results

Common environment, cwd worktree `ui/`:

```sh
OTTO_E2E_SLOT=ux4access OTTO_E2E_PORT=7864 OTTO_E2E_PW_PORT=5364 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/debug/ottod
```

All browser commands use `npx playwright test`, `--workers=2` (identity-only red used 1), and explicit outputs below. Two-engine runs specify `--project=desktop-browser --project=iphone-portrait`; red runs specify desktop-browser only.

- Prior: `e2e/desktop-ux-r1-access.spec.ts e2e/desktop-ux-r2-access.spec.ts e2e/desktop-ux-r3-access.spec.ts --output=/tmp/otto-ux-r4-access-prior` → 75 passed / 1 transient failure; `/tmp/otto-ux-r4-access-prior.log`.
- Initial R4 red: `e2e/desktop-ux-r4-access.spec.ts --output=/tmp/otto-ux-r4-access-red` → 6 failures, fixture distinction above; matching `.log`.
- Extended red: same spec `--grep 'late workspace|steps announce|contrast' --output=/tmp/otto-ux-r4-access-deep-red` → 7 failures, including 5 incorrect pre-auth confirmation fixture calls; matching `.log`. An initial invocation from repository root stopped with missing Playwright project before tests, then reran correctly from ui.
- Identity red: same spec `--grep 'effective-user' --output=/tmp/otto-ux-r4-access-identity-red` → 1 expected failure; matching `.log`.
- First repaired full R4 two-engine suite: same spec `--output=/tmp/otto-ux-r4-access-green` → **26 passed**; matching `.log`.
- Accent red + isolated old Skills retry: `e2e/desktop-ux-r4-access.spec.ts e2e/desktop-ux-r1-access.spec.ts --grep 'actual Markdown|pro-dark-tablet' --output=/tmp/otto-ux-r4-access-accent-red` → 5 expected contrast failures, unchanged Skills case passed; matching `.log`.
- Context/path red: R4 spec `--grep 'cross-workspace|directory stays' --output=/tmp/otto-ux-r4-access-context-red` → 2 expected failures; matching `.log`.
- Combined R1/R2/R3/R4, both engines: all four spec paths `--output=/tmp/otto-ux-r4-access-final` → **114 passed / 2 WebKit color-parser errors**; `/tmp/otto-ux-r4-access-final.log`. All 76 prior tests passed, and all product assertions outside the two helper errors passed.
- Final expanded R4 suite, both engines: `e2e/desktop-ux-r4-access.spec.ts --output=/tmp/otto-ux-r4-access-verified` → **42/42 passed (1.5m), exit0**; matching `.log`.
- `node --test unit/ambient.test.ts unit/asyncOwnership.test.ts` → **10 passed**, `/tmp/otto-ux-r4-access-units-final.log`. This includes existing all-theme generated/photo backdrop luminance and text-on-glass calculations. Earlier async-only subset passed4.
- `npx tsc --noEmit -p tsconfig.e2e.json` → passed, `/tmp/otto-ux-r4-access-types-final.log`.
- `node scripts/ui-guards.mjs` → passed, 804 files, no ratchet regression, `/tmp/otto-ux-r4-access-guards.log`. No baseline increases. `git diff --check` passed.

## Rendered evidence actually viewed

- Prior-run Native-light phone login, Pro-Dark OTP error, Warm-light phone RTL long Markdown, plus earlier fresh five-theme setup/choice sheets: inspected Native light/dark, Warm light/dark, Pro Dark at phone/tablet/desktop widths. Original custom-accent setup images revealed the reversed path and almost invisible focus boundary; these were not accepted as final.
- Red Native-light actual Markdown/link/input-focus screenshot from `...-accent-red` compared with corrected Native-light phone and Pro-Dark phone `accent-link-focus.png` from `...-final`. The corrected hue remains distinct, the link reads clearly, and the input boundary is visible while primary text stays readable.
- Final new screenshots: Parent independently viewed final Native-light and Pro-Dark phone accent-link/focus screenshots and Warm-light RTL first-workspace screenshot in `/tmp/otto-ux-r4-access-verified/`; the directory remains left-to-right and focused boundaries are visible.

Setup/confirm screenshots use synthetic accounts and paths. Auth writes are intercepted. Markdown accent checks deliberately render the actual shared renderer into a real Modal as a component fixture. Generated wallpaper is enabled and disabled with the actual UI preference; sheets remain opaque. Their text contrast is measured against real computed solid backgrounds, not inferred from token names. These measurements do not claim full application chrome contrast for every wallpaper pixel; the independent existing ambient unit matrix covers that color contract.

## Scores (reviewed surfaces only)

| Page family / variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
|---|---:|---:|---:|---:|---:|
| Login: five themes, RTL, invalid→corrected credentials | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Boot/offline recovery and stable focused Retry | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| First-account: all five themes/custom accents, keyboard steps, long tools, failure/Skip/lost response | 9.5 | 9.6 | 9.5 | 9.6 | 9.5 |
| Guest OTP: five themes, throttle/resend/verify | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Guest editor/viewer: live synthetic terminal, token/navigation races, ended/reload/invalid | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Shared long Markdown and choice sheets: five themes, phone/tablet RTL | 9.5 | 9.6 | 9.6 | 9.5 | 9.5 |
| Nested/async/removed/inert-trigger focus and delayed intrinsic preview | 9.5 | 9.6 | 9.6 | 9.5 | 9.5 |
| Custom-accent links/focus, generated wallpaper/reduced transparency/reduced motion | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Workspace/effective-user transition behavior | — | 9.5 | — | 9.5 | — |
| Browser JSON LSP initialization/diagnostics, independently rerun | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |

Scores are scoped to the described executed flows, not a whole-app certification. Deductions from 10: long choice sheets still pack dense technical content into small phone space; the first-account long-tool step requires intentional vertical scrolling in a 400px-high viewport; guest terminal touch/OS keyboard composition is not exhaustively measured. Workspace transition scores concern behavior, so no visual score is invented for that store. No arbitrary score ceiling is imposed for safely mocked writes.

## Remaining scope and R5 targets

- Physical VoiceOver/native OS chooser and software keyboard occlusion are unverified; browser focus/roles are verified. No production OTP email/account creation or provider-home modification occurred.
- Parent's R3 production-worker fresh-load/reload/actual HTTP outage/recovery evidence is accepted as separately owned evidence. This reviewer did not repeat it. The optional two-different-asset-version deployment remains unverified.
- R1 Skills wrong initial preview occurred once; isolated retry and later full rerun passed unchanged. Parent owns the Skills source and should carry the original trace into the final round; do not label it repaired by these shared changes.
- Useful deeper R5 candidates (not findings): same effective-user ID with changing token scopes, failed workspace-list load recovery during identity changes, deeply nested independently constrained editor/media descendants, and native screen-reader announcement order. No unverified defect is asserted for them.

Owned production files: `ui/src/lib/accent.ts`, `ui/src/lib/stores/ui.svelte.ts`, `ui/src/app.css`, `ui/src/lib/dialogFocus.ts`, `ui/src/lib/components/Modal.svelte`, `ui/src/lib/stores/workspace.svelte.ts`, `ui/src/modules/settings/Onboarding.svelte`, plus only the auth workspace reload script hunk in shared `ui/src/shell/App.svelte`. New `ui/e2e/desktop-ux-r4-access.spec.ts`; two guideline documents updated. Parent owns all commits/global gates.

Parent full UI check passed with zero errors/warnings; all454 unit tests passed (`/tmp/otto-ux-r4-handoff-check.log`, `/tmp/otto-ux-r4-handoff-unit.log`). The user subsequently made round4 final; the R5 suggestions above are remaining coverage candidates, not an authorized next round.
