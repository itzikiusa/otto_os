# Round 3 reviewer 5/10 — shared access, authentication and guest sharing

Worked only in `/Users/itziklavon/otto-ux-audit-20260925`. Read the protocol, R1/R2 access reports, parent pointers, AGENTS/design/accessibility/review guidance. Used systematic-debugging and test-driven-development skills. No commits, subagents, full npm check, actual OTP/email sends, provider-home writes or account mutations. Auth/share/onboarding writes were intercepted fixtures. The Settings file-import confirmation was canceled; an explicit intercepted import endpoint also prohibited writes.

## Independent prior verification

R1 + R2 accessibility specs passed **32/32**, Chromium desktop-browser and iPhone WebKit: Markdown bidi/logical spacing, long inline paths, keyboard-scrollable fences/tables/sheets, five themes, stacked drawer/sheets, pointer and keyboard focus, and browser LSP initialization/diagnostics. I viewed the newly captured Warm light phone RTL Markdown, Native dark short sheet, and Pro Dark tablet RTL Markdown. Prior fixes held. After the new dialogFocus change, all eight existing nested/pointer focus cases were rerun and passed in both engines (included in final 12-test run).

LSP production-bundle verification was **not rerun by this reviewer**. R2's recorded production result is not represented as my run. Current LSP dev transport regression passed independently. Modal delayed intrinsic-size overflow remains a candidate, not a confirmed failure: the actual image-in-Modal caller examined gives its preview fixed 72×72 dimensions, and prior overflowing text-body regressions passed. No speculative observer rewrite.

## Confirmed findings and repairs

All P2; file lines refer to final source unless otherwise stated.

1. **Old share replies replace a newer guest view.** `ui/src/modules/share/SharePage.svelte:57`, `:66`, `:147`, `:178`, `:220`. Intercepted old metadata, then navigated to a new link. Releasing the old reply replaced the title with `Synthetic old` and could restore Editor UI. Old access recheck subsequently hid the new session behind OTP; old OTP errors survived into a revisited link. Added request generation + captured session/token checks across metadata, role, OTP, resend and rechecks; reset link-local forms and read-only default; invalidate on destruction. Verify/resend serialize so they cannot invalidate one another's active code. Regression proves delayed metadata, whoami, OTP, resend, and access recheck cannot mutate a different link.
2. **A replacement token for the same session does not refresh permissions.** `ui/src/lib/router.svelte.ts:21`. Loading `#/s/same/token-editor`, then `#/s/same/token-viewer` left “You can type” displayed in both engines. The plain Map did not invalidate SharePage's derived token because sessionId was unchanged. A SvelteMap makes replacement reactive while keeping tokens memory-only and stripping them from history as before.
3. **Failed impersonation capability load displays the previous identity's grants.** `ui/src/lib/stores/auth.svelte.ts:75`. Root booted with synthetic git admin grants; impersonation switched to a non-root identity whose `/auth/capabilities` returned 503. `auth.can('git','admin')` wrongly stayed true. Clear grants before loading and accept the response only for its captured token. The server remains the authority; this repair prevents misleading UI affordances.
4. **Offline polls remove the useful recovery screen and its focused Retry.** `ui/src/lib/stores/auth.svelte.ts:96`, `ui/src/App.svelte:25`. Hold the second `/meta` request after initial failure: the Retry button disappears while waiting, dropping focus. Quiet retry preserves the offline view; a pending boot guard prevents duplicate concurrent polls/manual attempts. Loading/recovery copy has status semantics. Recovery to Login is verified.
5. **Login failure is not announced.** `ui/src/modules/settings/Login.svelte:47`. Five-theme invalid-credential fixture showed visible error but no alert/live region. Added `role=alert` and semantic danger text. Axe including contrast passes on actual rendered login cards in all five themes; the old contrast itself was not asserted as failing.
6. **First-account setup leaves the wizard before optional workspace creation finishes.** `ui/src/modules/settings/Onboarding.svelte:16`, `:49`. Root creation succeeds, workspace creation returns 503: the wizard unmounts and its error is lost. Keep the new account result/token while finishing workspace setup, transition to the authenticated shell only after success, and retry without recreating root. The recovery test verifies exactly one root POST and two workspace attempts. Credentials cannot be revisited after the account already exists. Setup error has alert semantics.
7. **Long detected tool versions overlap adjacent rows.** `ui/src/modules/settings/Onboarding.svelte:234`, `:346`. A 375×400 RTL screenshot visibly stacked three multiline versions despite the first horizontal-only assertion passing. Strengthened the regression to measure vertical overflow; it failed by 23px in both engines. Rows now grow, versions wrap, tool identities stay LTR, icons retain size, and the setup card scrolls from a safe top edge. Corrected screenshot visually inspected; every line is readable and Back remains reachable after scrolling.
8. **Async file-import confirmation loses its visible trigger in WebKit.** `ui/src/lib/dialogFocus.ts:3`. Real Settings → Import settings button → synthetic file picker → async file.text → confirmation → Escape returned focus elsewhere. Preserve the activating control until focus/another interaction moves on, consume it when the dialog opens, and ignore synthetic hidden file-input clicks. Both engines now restore Import settings; existing pointer and keyboard nested dialogs still pass.

## Executed evidence and commands

Unless explicitly noted, cwd was worktree `ui/`. Common environment:

```sh
OTTO_E2E_SLOT=ux3access OTTO_E2E_PORT=7854 OTTO_E2E_PW_PORT=5354 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- `npx playwright test e2e/desktop-ux-r1-access.spec.ts e2e/desktop-ux-r2-access.spec.ts --project=desktop-browser --project=iphone-portrait --workers=2 --output=/tmp/otto-ux-r3-access-prior` → **32 passed**, `/tmp/otto-ux-r3-access-prior.log`.
- New R3 suite, Chromium, before fixes → **11 failed / 5 passed**, `/tmp/otto-ux-r3-access-red.log`, `/tmp/otto-ux-r3-access-red`. Failures were stale sharing, capability inheritance, offline focus, five login alert variants, and first-account recovery. OTP recovery/contrast already passed.
- Corrected setup fixture supplies `/auth/me` for its new synthetic account (avoids unrelated invalid-fixture-token logout); setup-only run still reproduced lost workspace error, **1 failed / 1 passed**, `/tmp/otto-ux-r3-access-setup-red.log`. A first command from wrong cwd failed before tests with missing project; reran from ui.
- Initial fixes, full new suite both engines → **34 passed**, `/tmp/otto-ux-r3-access-green.log`, `/tmp/otto-ux-r3-access-green`.
- New vertical-overflow/token-replacement/async-focus cases plus delayed resend-role checks → **5 failed / 3 passed**, `/tmp/otto-ux-r3-access-deep-red.log`, `/tmp/otto-ux-r3-access-deep-red`. Actual async focus failed only WebKit; token replacement and vertical overlap failed both.
- Expanded suite after all fixes → **40 passed**, `/tmp/otto-ux-r3-access-final.log`, `/tmp/otto-ux-r3-access-final`. This invocation used root cwd with `ui/node_modules/.bin/playwright test --config=ui/playwright.config.ts ui/e2e/desktop-ux-r3-access.spec.ts --project=desktop-browser --project=iphone-portrait --workers=2 --output=/tmp/otto-ux-r3-access-final`. It safely ran the isolated daemon, then its exact generated root `e2e/.auth-ux3access/{state,daemon}.json` files and empty directories were removed after teardown. No credentials printed or staged.
- Final targeted existing focus regressions plus new authenticated-login and guest editor/viewer/end/reload/invalid flows: `npx playwright test e2e/desktop-ux-r1-access.spec.ts e2e/desktop-ux-r2-access.spec.ts e2e/desktop-ux-r3-access.spec.ts --project=desktop-browser --project=iphone-portrait --workers=2 --grep 'nested folder sheet|phone Navigator|pointer-opened|pointer drawer|guest editor input|login recovers' --output=/tmp/otto-ux-r3-access-focus-final` → **12 passed**, `/tmp/otto-ux-r3-access-focus-final.log`. Terminal WS is synthetic: Editor sends `hello`, Viewer sends no `blocked` input, output remains visible, ended card reloads, tokenless link explains invalidity.
- `npx tsc --noEmit -p tsconfig.e2e.json` → passed, `/tmp/otto-ux-r3-access-types-final.log`.
- `node scripts/ui-guards.mjs` → passed, 803 files, no ratchet regression, `/tmp/otto-ux-r3-access-guards-final.log`. No baseline increase. `git diff --check` passed.
- Final focused login run logs a Svelte `await_reactivity_loss` warning in `WorkspaceStore.load`, `ui/src/lib/stores/workspace.svelte.ts:434`; reported to parent, outside owned edits. No related assertion failure. Full npm check belongs to parent; their first wave had four WorkflowsPage typing errors, not in this scope.

## Screenshots actually inspected

- Current-run prior evidence: Warm light phone RTL Markdown; Native dark short focused sheet; Pro Dark tablet RTL Markdown (`...-prior`).
- Red login Native light; red OTP Warm light (`...-red`).
- Red first-account long tool versions (`...-setup-red`): obvious text overlap prompted stronger regression.
- Final Login **all five** theme/scheme images: Native light 375×667, Native dark 1000×667, Warm light 375×667 (WebKit), Warm dark 834×667, Pro Dark 1024×667. These were RTL and reduced-motion fixtures.
- Final OTP **all five** images: Native light 375×667, Native dark 1000×600, Warm light 375×480, Warm dark 834×768, Pro Dark 1024×768; RTL/reduced motion. Scoped axe including contrast returns no WCAG A/AA violations. Computed readable p/button/input font sizes are all ≥11px; icon glyphs are excluded.
- Final corrected 375×400 RTL long tool versions (WebKit); first-account workspace error 375×667; offline recovery with visible Retry focus (WebKit default device viewport); synthetic loaded viewer output on Native light phone; ended/reload card with synthetic terminal output (`...-focus-final`, WebKit).

All publication-suitable screenshots in the new auth/guest suite use synthetic data. The isolated daemon usage tailer reads host metadata; these screenshots show none of it. Parent chooses durable PR evidence.

## Scoped scores / 10

Numbers assess inspected surfaces, not whole application or untested combinations. No known confirmed issue remains in the repaired and tested paths.

| Page family / variant | Layout/readability | Interaction | Accessibility | States/recovery | Responsive |
|---|---:|---:|---:|---:|---:|
| Login, five themes/RTL, error and corrected sign-in | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Boot/offline and recovered Login | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| First-account wizard, normal and long tools, workspace failure/retry | 9.5 | 9.5 | 9.4 | 9.5 | 9.5 |
| Guest OTP, five themes/RTL, validation/throttle/resend/verify | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Guest viewer/editor, link/token races, ended/reload/invalid | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Shared Markdown and long choice sheets, prior five-theme regressions | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |
| Nested dialog and async file-confirmation focus, both engines | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 |
| Broader removed/inert-trigger and delayed intrinsic Modal content variants | — | 9.3 | 9.3 | 9.3 | — |
| Browser JSON LSP init/diagnostic surface, prior regression rerun | 9.5 | 9.5 | 9.5 | 9.5 | 9.5 |

First-account accessibility retains a deduction because its full step-to-step keyboard announcement/focus sequence was not deeply audited; this round verified field/action flow, alerts, recovery and clipping. The broader Modal row explicitly keeps an evidence gap visible rather than borrowing the tested stack's score.

## Limits and next fresh review

- No native Tauri/physical VoiceOver, production cloud mutations, actual email OTP, OS zoom or software-keyboard occlusion. Fixtures are sufficient evidence for UI request/recovery semantics, not those external integrations.
- First-account five-theme matrix, password-strength/mismatch announcement, every step's focus handoff, and failed-setup Back/Skip variants remain useful R4 targets.
- Custom accents/reduced-transparency contrast, removed/inert dialog triggers, and intrinsic media changes inside an already height-capped Modal remain unverified variants. No confirmed defect is asserted for them.
- Production PWA service-worker enabled offline/cache behavior is not covered by serviceWorkers:block fixtures. Parent already assigned later production review.
- Canvas queued-save/current-identity concern was sent to parent: the queue originally called api.put only after awaiting an older request, potentially picking up a new bearer. Parent took ownership, reproduced and repaired with a separate both-engine suite. I did not edit Canvas or claim that verification as mine.
- Owned production files are seven (`App.svelte`, `auth.svelte.ts`, `Login.svelte`, `Onboarding.svelte`, `SharePage.svelte`, `router.svelte.ts`, `dialogFocus.ts`) plus new `ui/e2e/desktop-ux-r3-access.spec.ts`. No Modal, Markdown, Terminal, OutputsPanel, FirstRunCoach or Canvas source edits.

Final offline bounds follow-up: **2/2 passed** in Chromium/WebKit, including fully visible recovery copy and no horizontal document overflow, `/tmp/otto-ux-r3-access-offline-final.log`, output `/tmp/otto-ux-r3-access-offline-final`; command used new spec `--grep 'offline automatic'` with the same two projects and slot. Final `.last-run.json` files for the 40-test, 12-test and 2-test runs report `passed`, `failedTests: []`. All test processes finished and teardown completed.

R4 follow-up: reproduce `await_reactivity_loss` at `WorkspaceStore.load` line 434 through real sign-in/identity transition; inspect whether the dependent workspace/session effect refreshes correctly after delayed API completion. The current warning alone does not establish harmlessness or a user-facing defect. Native OS file-chooser focus and physical VoiceOver are unverified; the passing file-import test drives Playwright's browser filechooser event with a synthetic file. Parent independently repaired Canvas identity queue handling in commit `82ce4ab5` and reports its own both-engine validation.

All five rounds remain mandatory.
