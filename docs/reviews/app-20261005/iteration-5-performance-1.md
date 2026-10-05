# Iteration 5 — performance partition 1

**Verdict: Approve the inspected repairs; performance acceptance remains pending.** Blocker 0 · major 0 · minor 0 · nit 0. **Provisional score: 9.4/10.** No new proven hot-path finding.

Source: `418e963a` plus the working-tree History lifetime/context repair inspected on 2026-10-05. Bounded follow-up to iteration-4-performance-1.md: repaired child transcript admission, pagination, release, and adjacent History navigation. This is not a fresh exhaustive shell/session/backend sweep. Applied the performance-review skill and read its cost catalogue, severity reference and finding template. The broad hotspot sweep was deliberately omitted under the assigned bounded-follow-up constraint. No builds, tests, benchmarks, source edits, git mutations or measurement runs were performed by this reviewer; only this report was written.

## Repair assessment and cost model

**P1-R4-01 is closed at source and mapped store-regression scope.** `ui/src/lib/stores/transcript.svelte.ts:141–153` establishes window-wide limits of eight admitted child bodies, 32 MiB aggregate charged payload and 8 MiB per accepted child page. `:477–483` reserves before fetching, so 100 simultaneous expansion attempts cannot launch 100 response downloads. Each accepted child retains one page (`:512`), rather than all pages traversed; grouping/render input is normally 60 turns per page. The adaptive oversized-page loop (`:490–499`) halves the requested limit, bounding retry count logarithmically in the initial 60-turn limit. It is deliberate bounded work, not a newly demonstrated redundant-fetch defect.

`acquireSubagent` shares one body between consumers; final release aborts and deletes payload and its charge (`:418–439`). The current-reader/epoch predicate (`:470`) and controller-owned cleanup (`:520–523`) prevent a late released response from restoring a body or releasing a newer reservation. `SubagentCard.svelte:36–44` returns the release function from the expansion effect, including unmount cleanup. At S sequential inspected children and P visited pages, retained body payload no longer scales as S×P×B: admitted bodies are capped by count and charge, while cursor metadata grows with pages visited. Charge is an accounting estimate, not a claim that process RSS stays below 32 MiB; response parsing and native allocation remain separate.

Mapped tests in `ui/unit/transcriptLifecycle.test.ts:161` cover 100 sequential inspections and reopening; `:174` covers a 1,200-turn child's older/newer reachability; `:192` covers collapse during a read; `:201` covers admission shared across parents; `:212` covers duplicate consumers; `:226` covers pre-fetch admission; `:248` covers replacement-byte accounting/refused-page retry; `:278` covers adaptive reachability; `:322` covers collapse/reopen late-response ownership. The verification ledger records the earlier focused red/green sequence and the root's current merged 1,333/1,333 unit pass. These are functional/accounting assertions, not browser heap measurements.

The working-tree History change (`ui/src/modules/agents/history/HistoryPage.svelte:67–79,172–196`) adds constant-size lifetime/context generation checks around existing async operations and clears the search timer on destruction. It introduces no per-row read or additional resume request. Suppressing departed completion avoids unnecessary navigation/mount work. Root reports the mounted departed-resume RED→GREEN and all 15 distinct merged desktop cases now passing, including the repaired mixed-batch selector rerun. This supports the inspected lifecycle boundary; it does not measure navigation latency under concurrent terminal output.

Previously documented terminal park/adopt, transcript recovery/tail and History query bounds are carried forward as baseline evidence only; this brief pass did not re-prove every unchanged implementation.

## Evidence limits and required acceptance cases

Current merged UI check is 0 errors/0 warnings; root reports 1,333 units, production build and unchanged bundle budgets green. All 15 distinct merged desktop cases have passing executions, rather than one pristine 15/15 invocation. Prior full Rust execution was 4,694 pass / 2 fail / 86 skip; both failed tests subsequently passed individually. The remaining full integration gate is pending. Other partitions' repair outcomes do not certify this partition's performance.

**Current-revision CPU/RAM/latency at N=0/1/3/5 concurrent sessions, sustained memory, and read-only real-app/native-child measurements are pending.** Prior app-20261004 scale and 15-minute sustained evidence receives limited credit: it exercises ordinary terminal/conversation switching, not the newly bounded child-heavy journey. Its reported renderer RSS increase with stable heap/DOM/listeners remains unattributed. No new leak or resolution is inferred.

Named acceptance cases:

- **P1-M1 child allocation recovery:** one mounted parent, 100 expand/collapse operations, then 1,200-turn older/newer paging; capture charged bodies, mounted turns, heap and process-group RSS after warm-up and recovery. Repeat with three parents; include budget rejection, retry and collapse/reopen.
- **P1-M2 responsiveness:** at N=1/3/5 emitting sessions, measure expansion/page/navigation p95 and main-thread long tasks while switching History/Agents and parking/adopting terminals. Functional green does not establish responsiveness.
- **P1-M3 representative measurements:** repeat isolated N=0/1/3/5 CPU/RAM/latency on current source, sustained run plus equal-duration idle control, and recovery beyond park/tail TTLs. Sample the real app and native WebKit children read-only; distinguish allocation growth from transient RSS/allocator retention.

## Fixed-rubric scores

| Dimension | /2 | Evidence and concrete deduction |
|---|---:|---|
| query/network work | 2.0 | Strong direct source plus mapped executed admission tests over this bounded repair matrix: capacity reserved before fetch, shared consumers, at most eight admitted child reads, no History request multiplier. No known material gap in that stated matrix; not a fresh deep-history query-plan claim. |
| algorithmic/serialization cost | 2.0 | One-page replacement prevents cumulative body copying/grouping; adaptive page reduction and older/newer reachability have mapped executed cases. At most eight charges are summed. No demonstrated superlinear cost in inspected repairs. |
| retained memory/lifecycle | 1.9 | Explicit release, abort, byte/count and late-reader ownership pass mapped store regressions. Deduct 0.1 for P1-M1: mounted child allocation/recovery has not yet been measured; accounting alone does not establish actual browser release. |
| scheduling/responsiveness | 1.9 | Pre-fetch admission and constant-size History ownership checks are directly supported; mounted departure behavior is green. Deduct 0.1 for P1-M2: bounded missing evidence for interaction latency while multiple sessions emit output. |
| representative CPU/RAM/latency measurements | 1.6 | Preserve limited prior completed scale/sustained evidence, with ordinary terminal/conversation workload mapping. Deduct 0.4 for P1-M3: no current-revision scale/sustained measurement, child-heavy measurement or complete native process attribution. This is not reset to zero, nor upgraded by unit counts. |
| **Total** | **9.4/10** | **Provisional; below 9.8 because specifically named measurement acceptance cases remain open.** |

The iteration-4 original 8.7 remains unchanged in its report. Root may append a separately labelled rescore after executing the acceptance cases above. No out-of-scope defect asserted.
