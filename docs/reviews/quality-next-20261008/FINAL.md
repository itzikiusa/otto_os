# Quality campaign — five complete review and fix rounds

Baseline: `7cff9e538deccdd0e346af7c6285e11863285c00`. Branch: `review/quality-20261008-next`. One pull request: [#97](https://github.com/itzikiusa/otto_os/pull/97).

Twenty unique reviewers completed five full rounds. Each round covered performance, correctness/bugs, design and UX; confirmed findings were repaired and reassessed. The final assessments are **9.8/10 in all four verticals for the reviewed scope**. No confirmed finding remains open in that scope. Scores express engineering judgment; they are not a measured whole-product reliability percentage.

## Review ledger

| Full round | Unique reviewers added | Performance | Correctness | Design | UX | Record |
|---|---:|---:|---:|---:|---:|---|
| 1 | 1–9 | 9.2 | 9.4 | 9.3 | 9.2 | [Historical checkpoint](VALIDATION.md) |
| 2 | 10–12 | 9.8 | 9.8 | 9.8 | 9.8 | [Round 2](rounds/round-2-coordinator.md) |
| 3 | 13–15 | 9.8 | 9.7 | 9.8 | 9.8 | [Round 3](rounds/round-3-coordinator.md) |
| 4 | 16–18 | 9.8 | 9.8 | 9.8 | 9.8 | [Round 4](rounds/round-4-coordinator.md) |
| 5 | 19–20; reviewer 12 returned | 9.8 | 9.8 | 9.8 | 9.8 | [Round 5](rounds/round-5-coordinator.md) |

The original five targeted stages collectively count as round 1. They are not five full rounds. Later independent passes found concrete defects despite earlier passing assessments, including the round-3 promotion persistence deduction, closed in round 4. Every final deduction/coverage boundary has a concrete explanation in its specialist report; no score was rounded up to meet the target.

## Resulting behavior

- Browser transport, request workers, proxy sockets and failed-origin history have explicit bounds and teardown ownership. Cached safe-method bursts remain responsive, and real Chromium can render and accept input through guarded positive HTTP bursts.
- Evaluation ratings preserve configured weights and original command evidence. Rating/retry publication is durable and serialized with promotion; malformed or incomplete verdicts cannot pass. Cancellation persistence errors are reported, and slow retry diff preparation does not block cancellation. Failed publication remains pending and recoverable; partial library/metadata publication is reported explicitly.
- Goal-loop settings validate before atomic persistence. Switching goals isolates answer/evidence drafts and dialogs. Vault refinement retains one admitted turn identity through completion and subsequent requests.
- Shared header focus, workspace recovery, Proof selection, chat completion and Find ownership remain consistent. Current scores refresh their evidence; pending scores do not present previous Approval artifacts as current. Recovery is visible and keyboard accessible in the checked desktop/phone and light/dark layouts.

## Final combined validation

Final source commit: `bc7a476e3e76f70bdb49176aa04e14bd52cfb706` (subsequent changes are review evidence/documentation). [Exact source/runtime manifest](evidence/round-5-coordinator/source-manifest.json). Fresh debug daemon SHA-256: `ecbcb15ff6a6f2fb6cc047d05cee32f31a1d01a66af035f3833d1f92cb17318f`.

| Final gate | Result |
|---|---|
| Stable toolchain update | Rust 1.99.0, unchanged |
| Workspace formatting and all-target Clippy with `-D warnings` | Passed |
| Workspace build | Passed |
| Workspace nextest, CI profile | **5,488 passed, 94 skipped**, 286.129 seconds |
| Documentation tests | Command passed; no runnable doctests, one ignored example |
| UI Svelte/TypeScript and style guards | Passed; zero errors/warnings |
| UI units | **1,828 passed**, zero failures/skips |
| Production UI build and bundle budgets | Passed |
| Final-daemon named browser integration | **41 passed**, zero failures/skips/retries |
| Nextest filter guard, Rust LOC ratchet, source/document whitespace | Passed |

Logs: [Clippy](evidence/round-5-coordinator/clippy.log), [workspace tests](evidence/round-5-coordinator/nextest.log), [doctests](evidence/round-5-coordinator/doctests.log), [build](evidence/round-5-coordinator/build.log), [UI check](evidence/round-5-coordinator/ui-check.log), [UI units](evidence/round-5-coordinator/ui-unit.log), [UI build](evidence/round-5-coordinator/ui-build.log), [bundle budgets](evidence/round-5-coordinator/bundle-budget.log), [filter guard](evidence/round-5-coordinator/filters.log), [browser](evidence/round-5-coordinator/browser.log), [browser results](evidence/round-5-coordinator/browser-results.json).

Browser execution used Node 26.10.0, one Chromium worker, isolated ports 17857/5230 and orphan sweeping disabled. Eight named specs cover all five rounds' UI repairs and Vault refinement. Both listeners were absent afterward ([cleanup](evidence/round-5-coordinator/cleanup.json)). Deliberate route-injected pending/error states are identified in the specialist reports; successful score publication and Vault calls use the real isolated daemon. This functional run overlapped workspace tests and makes no new timing claim.

The native standalone source was unchanged after its recorded 55-test/Clippy/probe checks. Native packaging remains a post-approval deployment step. The debug link emitted the existing macOS compact-unwind-size warning; build completion succeeded. Two coordinator command setup errors are retained separately: bundle checking first used the wrong working directory, and the filter guard first lacked nextest on PATH. Corrected invocations passed; neither changed source, thresholds or test expectations.

## Runtime and visual evidence

The independent final performance review rechecked cost bounds and audited prior executable/source hashes. The real Chromium positive-HTTP run delivered **384/384 uncached bodies**, zero network failures, input and fresh ACKed frames, followed by cleanup. The fresh round-4 WebKit/PTY run rendered **855/855 markers** with **18 flow-control acknowledgements**, p95 **26/34/36 ms** for one/three/five terminals, and empty queues after recovery. Those terminal/browser inputs are unchanged by round-5 evaluator fixes; they are historical measured artifacts, not claimed to be newly timed on the final daemon. [Performance assessment and exact limits](rounds/round-5-performance.md).

Native School evidence includes ten modal hide/resume cycles, bounded hidden rendering, fresh visible/unfocused frames and zoom/draft retention. Native source is unchanged since that probe. No current VoiceOver or signed-install acceptance is inferred from the browser or native probe. [Native/UI round-2 evidence](rounds/round-2-design-ux.md).

Current visual examples: [light desktop pending evidence](evidence/round-5-design-ux/pending-evidence-desktop-light.png), [dark desktop](evidence/round-5-design-ux/pending-evidence-desktop-dark.png), [dark phone](evidence/round-5-design-ux/pending-evidence-phone-dark.png), [light phone](evidence/round-5-design-ux/pending-evidence-phone-light.png). Earlier round reports retain populated API/database, conversation, goal-loop, Proof and shared-shell screenshots.

## Confidence and remaining boundaries

The reviewed scope covers changed code and traced consumers, evaluator failure/recovery and cancellation, browser admission/resource ownership, sampled terminal/transcript paths, and named populated UI/native journeys. Whole-product coverage is not established: every module, real provider inference, Internet TLS/HTTP2, sustained high-throughput saturation, hours-long leak testing, VoiceOver and all combined module workloads were not exhaustively exercised.

Library files and SQLite remain separate publication boundaries. An interrupted library write/metadata update may require manual reconciliation; the change reports partial success and provides retry guidance without overwriting potentially newer user content. Explicitly ignored/opt-in external-service and scale tests are not counted as executed by the default workspace suite. These limits remain visible rather than being converted into unexplained numerical deductions.

## Delivery

PR #97 is the only PR. Final local gates passed; hosted results belong to the final pushed PR head and are tracked on the PR. Advisory jobs remain reported separately and do not become new merge requirements. Merge, push to main, rebuild, reinstall and replacement of the running app are conditional on the user's requested approval and mandatory hosted gates. No production installation or real user state was modified during this review campaign.
