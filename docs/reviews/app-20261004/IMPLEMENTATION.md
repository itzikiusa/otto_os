# Implementation assignments and verification

The user authorized autonomous repairs and up to three iterations, followed by one PR and merge. Findings are evidence to reproduce and refine, not mandatory prescriptions. Keep the existing product and public contracts; no unrelated redesign. Prefer the smallest complete fix and meaningful failure/race regressions. Preserve other work and coordinate exact UI paths before edits.

Five implementation roles, run in waves of at most two shell-active agents. Parent owns builds, heavy tests, screenshots, load measurements, integration, and commits. Agents can add tests but must request the test slot before running commands that build or execute suites. No extra implementation subagents.

| Role | Scope and source reports | Main verification |
|---|---|---|
| 1 — session runtime | C1/P1/UX1 except Composer, plus shared bounded Folder snapshot used by P5-01 | Filesystem error vs absence, reconnect gap, captured upload destination, event synchronization; ring allocation cost; transcript paging/cache bounds; explicit load/search failure recovery |
| 2 — data tools | C2/P2/UX2 | Wrong-row mutation rejection, CH nonunique key rejection, exact staged/unstaged Git preservation, concurrent variable write sets; expanded Mongo byte/cell cap; capped SQL batches; bounded schema-history fetch; scratch/automation discard guards; live-tail failure/recovery |
| 3 — knowledge/product | C3/P3/UX3 + D3-04 | Story ownership and dirty transitions, atomic artifact publication/CAS, case-sensitive rename safety, tab annotation ownership, conflict retries, preview gate, persistence-aware Snip close/copy; bounded backlink contexts, slim version summaries, cached Vault counts, import retries |
| 4 — orchestration/MCP | C4/P4/UX4 | Requester-scoped approvals, atomic swarm dispatch lifecycle and reservation, workflow target/history ownership; bounded MCP process/buffering and scheduled output; batched capability/usage; remaining UX findings |
| 5 — assistant/platform | C5/P5/UX5 | Canonical decision handling, actual execution cancellation/takeover, plugin credentials lifecycle, Athena region; bounded Assistant cache and incremental indexing, Insights completion reads; remaining UX findings |

Visual/accessibility work stays with Claude. Verify merged fixes for D1, D2, D3 and later D4/D5 findings; reserve behavior changes separately. Role 1 owns Folder APIs; role 5 consumes them after coordination. Shared API types/contracts and integration test registry edits require coordinating narrow sections; migration numbers are reserved before adding files.

## Root runtime repair: K8s backfill

Confirmed in real read-only logs: large-day aggregation exceeds embedded ClickHouse memory and retries from the start indefinitely. Keep current restart-safe truncate-and-rebuild semantics. Bound query memory, read/insert blocks and aggregation spill; if needed split days into disjoint time windows so aggregate state remains composable. Avoid increasing the installed server's memory limit. Back off failed schema initialization and persist its error in monitor status. Validate a fresh migration, an interrupted/retried migration, exact aggregate counts, completion no-op and bounded peak memory against an isolated embedded server. No live DB mutation.

Alternative considered: resumable per-partition staging/checkpoints. That adds durable protocol and crash-consistency complexity across rollup tables whose partitions use different time grains. Defer unless bounded restart-safe work still cannot complete under the measured budget. Simply raising memory or suppressing errors would not fix the workload.

## Acceptance per fix

1. Trace the actual failing path and relevant existing defenses; reject false positives with evidence.
2. Add a regression for meaningful correctness/race/budget behavior, prove failure before the fix where feasible.
3. Implement, run focused checks centrally, record exact results and limits.
4. Independent original reviewers recheck fixes and adjacent paths in iteration 2; iteration 3 only for remaining findings/regressions.
5. Merge updated main preserving Claude's changes, verify combined UI and Rust gates, collect light/dark/responsive screenshots and performance measurements. Create one PR, wait for green CI, merge main.
