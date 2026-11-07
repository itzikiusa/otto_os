# Threads revival, responsive width, and goal/graph visuals

This follow-up makes stopped threads recoverable through an explicit Revive action, lets the conversation and composer use their pane width, and exposes engineering graphs alongside whole-goal loops.

## Behavior checked

- Dormant revival starts one provider runner even with concurrent callers, sends no prompt, preserves native conversation identity and persisted history, and leaves a live runner alone. The regression failed before the runtime fix and passed afterward.
- Closed runtime threads revive through the visible action without submitting the composer draft. Legacy stopped agent sessions use the existing idempotent resume endpoint.
- Wide conversations and split panes use their available width with matching composer gutters. Phone layouts remain within the viewport.
- Goal loop and Engineering graph are explicit execution choices, separate from Build/Research. Nodes explain phases or show task instructions, prerequisites, linked criteria and saved evidence.
- Graph edits survive mode toggles; regenerated AI drafts replace old task plans after confirmation. Removed criteria cannot return through the cached graph. Paused nodes never advertise active execution.

## Review

Independent correctness/UX review found no remaining must-fix or should-fix issues after correcting the browser error fixture and the stale graph-plan path. Keyboard navigation, visible focus, semantic colors, responsive layouts and short explanatory copy were checked against the UI checklist.

## Validation

Rustfmt, strict clippy and all 1,604 scoped backend tests passed (3 pre-existing ignored tests; doc tests had 1 ignored example). UI checks passed with zero errors/warnings; all 2,423 UI tests passed. Production build and bundle budgets passed.

Browser verification: all 5 existing runtime scenarios passed; 13 combined desktop/Android/iPhone scenarios passed, and the 6 graph cases were rerun successfully after the phone-header adjustment with explicit checks that Resume and the overflow menu are fully visible. Fixtures were corrected for the API error shape, deep-link lookup, and WebKit service-worker interception. The initial CSS compatibility warning and ownership fixture state name were fixed before the final gates. Browser fixtures isolate provider/network responses; backend tests exercise the real runtime with deterministic fake provider processes. Physical-phone/Tailscale and live-provider acceptance are not claimed.

## Screenshots

- [threads wide light](threads-wide-light.png)
- [threads phone light](threads-phone-light.png)
- [goal loop light](goal-loop-light.png)
- [engineering graph light](engineering-graph-light.png)
- [threads wide dark](threads-wide-dark.png)
- [threads phone dark](threads-phone-dark.png)
- [goal loop dark](goal-loop-dark.png)
- [engineering graph dark](engineering-graph-dark.png)
