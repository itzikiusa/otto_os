# Round 2 — reviewer 7/10 — shell / Home / Agents / panels

Worktree: `/Users/itziklavon/otto-ux-audit-20260925`. Read AGENTS.md, design guidelines and R1 shell report. No commits, subagents, original-checkout changes or real-data mutations. The isolated release daemon ran on 7842; Vite on 5342; slot ux2shell. Browser mocks block service workers.

## Prior fixes independently checked

All eight checks in `desktop-ux-shell.spec.ts` pass: Navigator focus trap/return, named shared prompt, Home shortcut isolation, palette focus/Escape, mixed Needs-you expansion, Native light/dark and Warm dark phone RTL. The original five R1 fixes remain effective. Viewed actual rendered Home evidence; initial legacy screenshots captured the dialog during its fade, so they are not publication evidence. New screenshots disable animations and show opaque, correctly focused dialogs.

The R1 residual Outputs error was reproduced: failed GET `/sessions/{id}/artifacts` rendered “Nothing produced yet” without Retry. Repaired below.

## Reproduced and repaired

1. **P1 — terminal copy lost its selection during background compact.** `ui/src/lib/components/Terminal.svelte:424,581`. Parent's core suite was 27 passed / 1 failed; independent rerun reproduced the permissionless-copy failure. Instrumentation showed an EMPTY xterm selection mirror before copy. When selection existed, native permissionless copy succeeded. The delayed resize compact called `term.reset()` while the user was selecting/pausing to copy. Browser shells take this DOM-rendered path too. Guard the request while selected; also guard a same-process compact response if selection began while it was in flight. Clear compact/epoch state on reconnect; new process epochs still rebuild. A real WebSocket response was held, text selected, response released: pre-fix mirror became empty; post-fix selection survived. Existing copy regression now pauses across the compact interval before copying. No permission handling was weakened.
2. **P2 — artifact list failure masqueraded as empty.** `ui/src/lib/stores/activity.svelte.ts:102`, `ui/src/modules/panels/OutputsPanel.svelte:215`. Added session-scoped loading and error state, inline error detail/Retry, and in-flight de-duplication. Loading is announced. The effect uses `untrack` so reading request state does not cause retry loops. Red test had no alert; green test recovers two artifacts without leaving the panel.
3. **P2 — switching a pending preview to a URL remained loading forever.** `OutputsPanel.svelte:100`. Reset loading for every new selection, including synchronous URL/history previews. Red screenshot visibly showed “Loading preview…” beneath selected Preview documentation; green test exposes its link immediately.
4. **P2 — reselecting the same artifact accepted an older response.** `OutputsPanel.svelte:101,121,155`. IDs alone did not distinguish first request A, select B, second request A. The old response overwrote the newer preview. Added per-request generation invalidated on session/artifact changes. Test controls two responses and proves the current heading survives the obsolete response.
5. **P2 — Outputs listbox lacked listbox keyboard interaction.** `OutputsPanel.svelte:167,238`. ArrowDown/Up and Home/End now move focus and selection; roving tabindex avoids tabbing through every row. Browser red test left focus on the first item; green test navigates/selects correctly.

Files changed: Terminal.svelte; activity.svelte.ts; OutputsPanel.svelte; existing desktop-terminal-copy.spec.ts (stronger assertion); new desktop-ux-r2-shell.spec.ts.

## Verification

All commands from WT/ui use:

```sh
OTTO_E2E_SLOT=ux2shell OTTO_E2E_PORT=7842 OTTO_E2E_PW_PORT=5342 OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=/Users/itziklavon/otto-ux-audit-20260925/target/release/ottod
```

- `npx playwright test e2e/desktop-terminal-copy.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-shell-copy`: **1 failed, 3 passed**, reproduced prior copy issue. Log `/tmp/otto-ux-r2-shell-copy.log`.
- First Outputs error/loading red run: **2 failed, 1 passed** (copy alone passed). `/tmp/otto-ux-r2-shell-red.log` and corresponding output directory. Failures on promised UX assertions.
- Further Outputs stale/keyboard red run: **2 failed**, `/tmp/otto-ux-r2-shell-deep-red.log`.
- Delayed snapshot red test: `/tmp/otto-ux-r2-shell-snapshot-red2.log`, **1 failed** at final post-response selection assertion. First attempt incorrectly counted attach frames and failed setup; corrected after inspecting server sends (initial screen binary, explicit initial snapshot, optional compact snapshot).
- `npx playwright test e2e/desktop-ux-r2-shell.spec.ts e2e/desktop-ux-shell.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-shell-fixed`: **15 passed (37.1s)**, including eight prior shell/Home checks and five-theme loaded Outputs.
- `npx playwright test e2e/desktop-terminal-copy.spec.ts e2e/desktop-conversation.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-shell-terminal-fixed`: **17 passed (1.1m)**. All 13 conversation checks: draft isolation, pending send draft safety, tool details, system toggle, view persistence, split rendering, composer, search, diff, slash completion, image attachments, keepalive. Four copy cases passed.
- `npx playwright test e2e/desktop-ux-r2-shell.spec.ts e2e/desktop-terminal-copy.spec.ts --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-shell-final`: **16 passed (43.2s)**. Final production fixes, delayed response, stale preview, keyboard, five themes, split and Home light/dark screenshots. `.last-run.json` says passed, no failed tests. Log `/tmp/otto-ux-r2-shell-final.log`.
- `npx playwright test e2e/sessions-mobile.spec.ts --project=iphone-portrait --project=ipad-portrait --workers=1 --grep 'session view:|pane header|viewing data:|writing commands:|tab bar:|tiled view|floating controls|drawers' --output=/tmp/otto-ux-r2-shell-mobile`: **13 passed, 3 pre-existing device skips (45.5s)**. Real WebKit layouts, output, session switching, tiles, iPhone controls/drawers and iPad terminal typing. Phone typing is intentionally skipped by the existing spec because soft-keyboard focus cannot be synthesized; two phone-only checks skip on iPad.
- `npx tsc -p tsconfig.e2e.json --noEmit`: passed, `/tmp/otto-ux-r2-shell-tsc.log` empty. Fixed initial test option typing (`contextOptions.reducedMotion`).
- `node scripts/ui-guards.mjs`: passed, no new ratchet debt. `/tmp/otto-ux-r2-shell-guards.log`.
- `git diff --check` on owned files: passed. Parent owns full npm check/build/global integration.

## Screenshots viewed

Viewed all five loaded Outputs images under `/tmp/otto-ux-r2-shell-fixed/desktop-ux-r2-shell-loaded-Outputs-*/loaded-outputs.png`: Native light/dark 1440×900; Warm light RTL 1024×900; Warm dark RTL 390×844; Pro Dark 1440×900. Clean bounded panel, readable report, truncated long titles with full hover title, internal scroll, accessible preview buttons. Final equivalent evidence is under `/tmp/otto-ux-r2-shell-final/` with animations disabled.

Viewed final settled Home and loaded split light/dark images:
- `/tmp/otto-ux-r2-shell-final/desktop-ux-r2-shell-loaded-dd96f-d-settled-Home-prompt-light-desktop-browser/{loaded-split,settled-home-prompt}.png`
- `/tmp/otto-ux-r2-shell-final/desktop-ux-r2-shell-loaded-45a66-nd-settled-Home-prompt-dark-desktop-browser/{loaded-split,settled-home-prompt}.png`

Viewed initial terminal copy failure and Outputs pending-link failure screenshots. All new screenshot content is seeded synthetic/redacted fixture data. Do not publish legacy Home screenshots taken mid-transition.

## Scores by surface and variant

Scores assess inspected UI and verified behavior; no aggregate conceals split readability.

| Surface / important variants | Layout/readability | Interaction | Accessibility | States/recovery | Responsive | Overall |
|---|---:|---:|---:|---:|---:|---:|
| Shell navigation / palette / prompts, desktop + phone RTL | 9.6 | 9.6 | 9.6 | 9.5 | 9.5 | 9.6 |
| Home populated synthetic cards / spaces / prompts, Native light/dark + Warm phone RTL | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 | 9.5 |
| Agents full terminal, desktop copy + WebKit iPhone/iPad controls/output | 9.5 | 9.6 | 9.5 | 9.5 | 9.5 | 9.5 |
| Agents loaded Chat / Split, light + dark | 9.1 | 9.6 | 9.3 | 9.5 | 9.3 | 9.3 |
| Agents tiled phone/tablet | 9.3 | 9.5 | 9.3 | 9.4 | 9.5 | 9.4 |
| Outputs loaded + error/retry + stale async + keyboard, five themes / phone / tablet RTL | 9.6 | 9.6 | 9.5 | 9.6 | 9.6 | 9.6 |
| Activity / Files panels | 9.3 | 9.3 | 9.2 | 9.2 | 9.4 | 9.3 |

## Remaining issues and limits

- **P2 readability — desktop Split can shrink terminal below 11px.** `ui/src/lib/components/Terminal.svelte:683–691` intentionally sets MIN_FIT_FONT=6 to keep ≥80 PTY columns. Actual 1440×900 Native dark split screenshot shows 10px terminal font with roughly 512px terminal region. This violates the readable-content floor; no speculative claim is needed. Parent explicitly reserved deeper sizing repair for the next fresh shell reviewer, preserving PTY-width/anti-corruption behavior with resize/reconnect/copy regressions. Do not simply raise the minimum blindly. The split/tile scores remain below target.
- No physical macOS/Tauri clipboard or VoiceOver run. Browser native-copy event verified; not native Edit menu dispatch.
- Fixture-backed chat tests do not exercise production providers/cloud writes. Phone soft-keyboard typing remains outside automated emulation evidence.
- Activity/Files have less fresh deep coverage than Outputs; no claim of exhaustive async workspace-switch, long file-tree, PDF/image or artifact download coverage.
- No whole-app completion claim. Full integration gates and remaining rounds belong to parent.

Final follow-up: `npx playwright test e2e/desktop-history-tasks.spec.ts e2e/desktop-session-commands.spec.ts --grep 'activity panel|outputs panel|reconnect|terminal fits|expanding the right' --project=desktop-browser --workers=1 --output=/tmp/otto-ux-r2-shell-panels`: **5 passed (27.1s)**. Activity add-task persisted across reload, real fixture Outputs markdown preview, exited-shell reconnect, correct terminal fit, and panel expansion preserved the terminal grid. Log `/tmp/otto-ux-r2-shell-panels.log`; `.last-run.json` passed. This confirms the final snapshot guard retains reconnect and resizing behavior. All my test processes completed; slot ux2shell released.
