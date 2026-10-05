# Iterations 4–6 findings tracker

Source baseline `03f2bc3e`. Findings need concrete traces/reproductions before implementation. Tests pending are not passing. See PLAN.md and SCORES.md for rubric and acceptance.

| ID | Finding | Ownership | State |
|---|---|---|---|
| R4-C1-01 | History Resume treats working session as restart, killing active PTY (blocker) | Codex role1; HistoryPage resume + HistoryStatus type | Source-confirmed; report ready; reserved with Claude |
| R4-C1-02 | History Open in Chat bypasses transcript view cache (minor) | Codex role1; HistoryPage open handler | Source-confirmed; report ready; reserved with Claude |
| C4-2-01 | Broker replay drops/corrupts binary key/value/header and null value semantics (blocker) | Codex role2; broker service/Kafka | Source-confirmed; report ready |
| C4-2-02 | Delayed DB guarded-write prompt names current connection B while retry writes captured A (blocker) | Codex role2; database store | Source-confirmed; report ready; reserved with Claude |
| C4-2-03 | Broker preview slices UTF-8 at arbitrary byte offset and can panic (major) | Codex role2; broker service | Source-confirmed; report ready |
| C4-2-04 | API late loader/mutation responses publish old workspace data into current workspace | Codex role2; API store | Source-confirmed; report ready; reserved with Claude |
| C4-2-05 | Environment Save response replaces edits made while Save was pending | Codex role2; EnvironmentsView | Source-confirmed; report ready; reserved with Claude |
| R4-C3-01 | Concurrent Design metadata patches lose acknowledged independent edits (blocker) | Codex role3; Design service/store | Source-confirmed; report ready |
| R4-C3-02 | Browser delayed tab creation crosses workspace ownership (major) | Codex role3; Browser store | Source-confirmed; report ready; reserved with Claude |
| R4-C3-03 | Browser navigation persists another selected tab's title (major) | Codex role3; Browser store | Source-confirmed; report ready; reserved with Claude |
| R4-C3-04 | Excalidraw restore omits persisted background/grid state (minor) | Codex role3; ExcalidrawCanvas | Source-confirmed; report ready; reserved with Claude |
| R4-C4-01 | Workflow graph/version/snapshot publication can execute a different graph (blocker) | Codex role4; workflow repository/routes | Source-confirmed; report ready |
| R4-C4-02 | Old scheduled completion consumes newly retimed one-shot (major) | Codex role4; scheduler repository/engine/routes | Source-confirmed; report ready |
| R4-C4-03 | Fired workflow trigger does not rearm on retiming (major) | Codex role4; workflow triggers | Source-confirmed; report ready |
| R4-C5-01 | Parallel personal-agent schedule/run loads overwrite other entries (major) | Codex role5; personalAgents store | Source-confirmed; report ready; reserved with Claude |
| R4-C5-02 | Stale Proof filter response replaces current rows/cursor (major) | Codex role5; Proof store | Source-confirmed; report ready; reserved with Claude |
| R4-C5-03 | Pending Proof refresh reopens closed detail (minor) | Codex role5; Proof store | Source-confirmed; report ready; reserved with Claude |
| P1-R4-01 | Collapsed subagent bodies accumulate outside active budgets; child paging mounts unbounded turns (major) | Codex role1; transcript store/SubagentCard | Sized source finding; runtime magnitude pending; reserved with Claude |
| R4-P2-PERF-01 | Aggregate Working diff retains every untracked patch before capping (major) | Codex role2; Git local diff | Sized source finding; runtime pending |
| R4-P2-PERF-02 | Workbench history reloads/mounts lifetime revision list (major) | Codex role2; Workbench repository/API/HistoryPanel | Sized source finding; runtime pending; reserved with Claude |
| R4-U1-01 | Mixed session creation drops failed requests' configuration (minor) | Codex role1; NewSession submit recovery | Source trace; reserved with Claude |
| R4-U1-02 | Search gives whole-conversation absence claim over loaded tail only (minor) | Claude; ConversationView scope feedback | Forwarded for external implementation |

Canonical IDs/severity/traces are in reviewer reports. No repairs or new runtime verification yet.

Claude owns iteration-4 visual/a11y/copy fixes and announced URL selection work in Workflows/Proof/Swarm/Scheduled/Loops/Product, plus notifications deep links and swarm bulk allSettled. External reports: `/tmp/otto-design-iter4-findings.md`, `/tmp/otto-design-review-brief.md`; coordination `/tmp/otto-app-review-coordination-20261005.txt`.
