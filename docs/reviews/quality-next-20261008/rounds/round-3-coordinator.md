# Full round 3 — integrated review and repairs

Starting commit: `d4b8626d9`. Reviewers 13–15 independently reviewed performance, correctness, design and UX. The earlier 9.8 judgments were challenged rather than treated as an acceptance shortcut. Confirmed issues were repaired and checked; correctness retains one explicit unfinished publication boundary for Round 4.

| Vertical | Scoped quality | Basis |
|---|---:|---|
| Performance | 9.8 | [Bounded failed-host history; real positive HTTP bursts](round-3-performance.md) |
| Correctness | 9.7 | [Cancellation, promotion synchronization, process recovery](round-3-correctness.md); promotion metadata failure remains |
| Design | 9.8 | [Provisional scores, current evidence, populated navigation](round-3-design-ux.md) |
| UX | 9.8 | [Rating recovery, inline proof Retry, workspace continuity](round-3-design-ux.md) |

These are scoped engineering judgments with confidence limits in the specialist reports. The campaign continues: the correctness deduction has a concrete error branch, a required injected-failure regression and a retry acceptance condition. No score is silently rounded up.

## Additional coordinator finding: Vault refinement never completes its registry turn

`crates/otto-server/src/vault_docs_agent.rs` admitted a refine turn with one ID, generated a second ID for its durable row, and shadowed the first variable. Both callbacks then supplied the second ID to a guard that requires the admitted ID. Consequently a successful request returned a session, while the registry retained no session and stayed running; the next refine request could return conflict.

The hosted named Vault test failed at the registry-session assertion ([failure excerpt](../evidence/round-3-coordinator/vault-hosted-red-excerpt.log), [exact head/job provenance](../evidence/round-3-coordinator/hosted-provenance.json)). The repair uses the admission ID for the durable row and both callbacks. Independent correctness review confirmed that reset and newer-turn guards still reject obsolete callbacks. The existing browser regression now also checks idle completion, a successful second request with coherent session registration, and a fresh binding after reset. Its stubbed agent emits IDs without persistent session rows, so it does not claim real CLI resume.

## Combined browser and UI verification

Fresh daemon SHA-256 `bd1e244b32d6895a7738a6dc8b4f0db79c19de0ef9482f90f609a27f2f795d34`, isolated port 17836, Vite 5211, Node 26, one worker, orphan sweep disabled. **16 named browser cases passed, zero skips/retries**: evaluator evidence/recovery, header/workspace, and Vault agent/refine journeys. All scoring success paths and Vault calls used the daemon; deliberately injected pending/error responses are identified in the UI report. [Browser log](../evidence/round-3-coordinator/browser.log), [results](../evidence/round-3-coordinator/browser-results.json), [runtime provenance](../evidence/round-3-coordinator/runtime.json).

**1,828 UI unit tests passed**, zero failures/skips ([log](../evidence/round-3-coordinator/ui-units.log)). Final UI type/guard check passed, including the final Proof auto-selection repair ([log](../evidence/round-3-coordinator/ui-check.log)). Final production build and affected Vault Rust verification are recorded in the closure addendum below.

Hosted header failures were traced to a test observing transient empty-state toolbar contents, followed by waiting for a More button after the remaining actions fit. Its fixture now opens a stable populated request and waits for its normal autofocus before changing toolbar focus. The same viewport, focus, overflow, menu and draft assertions remain; no timeout or production layout rule was weakened. The hosted jobs remain advisory; their actual findings were reviewed without making advisory status a new merge requirement.

All confirmed source repairs in this round were independently reread by their specialist/coordinator counterpart. Full workspace delivery gates follow after the final round. No installed app or live user data changed; PR #97 remains the sole draft PR.

## Closure addendum

Final production UI build passed ([log](../evidence/round-3-coordinator/ui-build.log)); affected Vault scope Rust tests passed ([log](../evidence/round-3-coordinator/vault-rust.log)). Source/document whitespace and Rust LOC ratchet passed. [Final source hashes](../evidence/round-3-coordinator/source-manifest.json). Round 3 is closed; R3-C3 moves explicitly to Round 4.
