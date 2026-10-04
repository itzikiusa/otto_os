# Iteration 2 integration review

Root spot checks supplement the independent reviewer follow-ups. Findings below were traced through current implementation code, not inferred from failing unrelated harnesses.

- **C4-R2-01 (high): stop blocked by dispatch mutex.** The initial orchestration fix acquired the per-swarm operation guard before session creation and retained it through `wait_for_tui` (90 seconds) and up to two `PROMPT_LAND_WAIT` waits (120 seconds each). Pause and Abort acquired that same guard, so a stalled initialization could prevent stop for over five minutes and serialize independent agent startup. Direction: hold only during checked session registration and individual checked input writes, release during readiness waits, interrupt waits from persisted terminal run state/cancellation. Implementation 4 is correcting this and adding a deterministic race test.
- **C3-R2-01 (medium): keyboard Copy retained the old no-op.** Snip's button called `copyNow(true)`, while the Command-C handler still called default `copyNow()`. A previously persisted unchanged image therefore was not recopied by the shortcut. Direction: pass explicit intent from both entry points; test the actual shortcut handler. Also ensure persistence drain returns failure when dirty annotations cannot flatten because no image is available. Implementation 3 is correcting both.

Verification caveats found during integration:

- Initial notification navigation red failed in a mock initializer (`ws.sessions` absent), not in the navigation path. Corrected fixture subsequently passed with the full 1,027-test UI suite. No behavioral-red claim remains.
- Initial History scroll E2E reached both earlier cursors but concurrent Vite HMR remounted ConversationView, causing new initial requests and dropping the reading window. Trace confirmed HMR messages. Rerun on HMR-disabled Vite passed with unchanged paging source.
- The design stale-snapshot test was added before its source fix, but compilation completed after the fix. It passed; source tracing supports the original defect, but this run is not red evidence.
