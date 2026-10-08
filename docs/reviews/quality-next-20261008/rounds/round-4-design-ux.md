# Round 4 — design and UX

Reviewer **18/20**. Starting commit `2913a7d17`; source and daemon hashes are recorded in the [manifest](../evidence/round-4-design-ux/manifest.json). Read the design guidelines and full review checklist, the Round 3 findings and fixes, evaluator publication/evidence UI, Proof workspace selection, and goal-loop page/detail/store ownership. This round independently retested the prior six evaluator/Proof journeys and broadened review to populated Goal Loops in the real app shell.

## Confirmed finding and repair

**R4-UX-1 · medium · fixed — another goal inherits the previous goal's unsaved decisions and budget dialog.** `ui/src/modules/loops/LoopsPage.svelte:120` rendered an unkeyed `LoopDetail`. Changing an existing direct link from `#/loops/alpha` to `#/loops/beta` reused that component's local state. Questions and human-verification criteria commonly reuse ids; the new goal displayed alpha's unsaved answer/evidence under beta's labels. An open Extend budget dialog also survived and now targeted beta with alpha's draft caps. This can mislead a person into recording evidence for the wrong goal or changing the wrong budget.

The [red run](../evidence/round-4-design-ux/loops-red.log) reproduced the inherited answer in both light and dark, and the surviving budget dialog (three failures; existing inline Retry behavior passed). The repair keys the detail by `selectedId`, giving each goal its own component lifecycle. No new store, styling, or design primitive was added.

Acceptance, all passed:

- Alpha's answer/evidence never appear in beta; record controls remain disabled until beta has its own input.
- Returning beta→alpha does not resurrect unsaved drafts. **Drafts are discarded when leaving a goal**, consistent with the existing Back-to-list behavior; this change does not promise draft persistence.
- Alpha's budget sheet closes on navigation; beta's new sheet uses beta's defaults. Keyboard Cancel restores focus to its trigger.
- A held DELETE for alpha, completed after beta is opened and its budget is edited, leaves beta's heading, route, open dialog and draft intact. The request targets alpha only. The test route fulfills the request; no real loop is deleted.
- An initial failed detail read shows shared inline Retry and recovers with Enter.

## Independent challenge of Round 3

All six existing evaluator/Proof cases passed again: real score-only human rating updates expanded Approval evidence in light/dark and phone layouts; pending score publication retains the rating and offers inline recovery; proof read failure recovers with keyboard Retry; a held old proof response cannot replace the new evidence; and a real workspace switch replaces the open Proof pack. I also traced the pending per-iteration Promote action: it deliberately opens the existing gate/explicit root-override flow, so its presence is not asserted to bypass the server gate. No additional confirmed defect was found in these surfaces.

## Verification and visual inspection

- **11/11 browser tests, zero skipped/flaky/unexpected:** five new cases in `desktop-quality-round4-loop-ux.spec.ts`, all six cases in `desktop-quality-round3-eval-ux.spec.ts`. [Final log](../evidence/round-4-design-ux/browser-final.log), [machine results](../evidence/round-4-design-ux/browser-final-results.json). The first combined ten-case pass and separate late-delete pass are retained as intermediate evidence, not extra independent review rounds.
- `npm run check`: **0 errors, 0 warnings**, including UI guards and E2E TypeScript. [Log](../evidence/round-4-design-ux/ui-check.log).
- `npm run build`: **passed**. [Log](../evidence/round-4-design-ux/ui-build.log).
- Opened and visually inspected Goal Loops at 1440×900 and 390×844 in light/dark, evaluator light/dark expanded evidence, and pending desktop/phone. Shared chrome, opaque evidence/forms, text hierarchy and native controls remain readable; no horizontal overflow; phone answer field fits. [Loop desktop light](../evidence/round-4-design-ux/loop-desktop-light.png), [desktop dark](../evidence/round-4-design-ux/loop-desktop-dark.png), [phone light](../evidence/round-4-design-ux/loop-phone-light.png), [phone dark](../evidence/round-4-design-ux/loop-phone-dark.png), [pending desktop](../evidence/round-4-design-ux/rating-pending-desktop.png), [pending phone](../evidence/round-4-design-ux/rating-pending-phone.png).

Tests used Node 26.10.0, Chromium desktop-browser, one worker, no retries, no orphan sweep, isolated daemon 17848/Vite 5221, slot `round4-ux`. Goal content and failures are deterministic HTTP route fixtures on a real isolated workspace and app shell. Evaluator healthy scoring/proof and workspace switching use the real isolated daemon; pending failure presentation is route-injected. No providers, installed app or port 7700 were used. The daemon hash is the Round 3 build `bd1e244b…`; this run does **not** claim to execute the concurrent Round 4 backend repair. The coordinator owns fresh-daemon integration. Compilers overlapped functional tests, so timings are not performance evidence. The first root-cwd invocation created owned auth files under root `e2e/.auth-round4-ux`; after teardown only those files were removed. Other agents' fixture directories were preserved.

## Separate reassessment

| Lens | Observed quality | Confidence |
|---|---:|---|
| Design | **9.8/10** | High for inspected light/dark desktop/phone goal, evaluator and evidence states; medium for whole-product generalization. |
| UX | **9.8/10** | High for named recovery, state-ownership and keyboard journeys, including concrete red→green regressions; medium outside this sampled scope. |

There is no remaining confirmed design/UX deficiency in this round's named scope. Unknown coverage is reported as confidence, not an invented numeric defect. Native VoiceOver, physical keyboard behavior, other theme combinations, every module, and real agent execution remain outside the claimed evidence. No acceptance check was weakened to reach the scores. Round 5 should challenge these ownership boundaries independently and retain the final fresh-daemon integration gate.
