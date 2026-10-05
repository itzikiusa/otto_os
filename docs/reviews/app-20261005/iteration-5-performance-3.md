# Iteration 5 — performance partition 3

**Verdict: Approve the inspected repairs; runtime performance acceptance pending.** Blocker 0 · major 0 · minor 0 · nit 0. Provisional **8.5/10**, compared with iteration 4's preserved 7.0/10. Source checkpoint `64a850e6`, worktree `/Users/itziklavon/claude_ade-review`.

Applied performance-review to the two prior findings and adjacent Product/Design Hall integrations. Read PLAN, VERIFICATION, the prior report, and the skill cost references. This deliberately bounded follow-up did not rerun the broad hotspot scan, tests, builds, benchmarks, or production sampling. Only this report was written. Vault, Canvas, Browser and Snip retain the previous bounded source coverage; they were not broadly rescanned.

## Prior findings and cost trace

**P3-01 closed for the ordinary draft overview.** `ui/src/lib/stores/product.svelte.ts:469` requests 50 metadata rows with `summary=true`. `crates/otto-state/src/product.rs:1795` clamps pages to 100, projects `octet_length(body)` rather than materializing bodies, and uses a `(created_at,id)` keyset. `crates/otto-state/migrations/0171_product_transcript_pages.sql:2` supplies the matching `(story_id,created_at DESC,id DESC)` index. Initial body payload falls from O(T × B) to zero body bytes, with O(P × metadata) response size, where T is historical transcript count, B average body bytes, and P≤100 (UI P=50). Metadata title size still contributes to response bytes; no fixed total-payload byte claim is made.

Individual body loading is deduplicated by ID; the retained body cache at `ui/src/lib/stores/product.svelte.ts:507` admits at most four bodies and 4 MiB of UTF-16 payload. Oversized bodies remain downloadable. This is a retained-cache bound, not a bound on transient fetch/parse memory or simultaneous distinct-body requests. The compatibility full-list method still exists, but the repaired overview no longer invokes it.

Adjacent explicit find remains functional without automatic full-history hydration. `crates/otto-state/src/product.rs:1834` scans at most 100 records per page with a streamed query, bounds returned matches, and yields between rows. The client at `ui/src/lib/stores/product.svelte.ts:547` follows pages with cancellation/ownership checks. A no-match explicit search can still read O(T × B) bytes and make ceil(T/100) requests; that is the requested whole-history search, not the old eager overview cost. The index avoids an unbounded sort on each cursor page. Runtime scan latency and cancellation under a large single body remain measurement gaps, not newly demonstrated defects.

**P3-02 closed for the all-changed amplification and unbounded mounted rows.** `ui/src/lib/components/DiffView.svelte:61` budgets DP cells and repeated-line comparisons at four million; `:111` strips equal prefix/suffix, uses ordered unique anchors, and falls back to a bounded-distance repeated-line diff. `:27` and `:327` page actual rendered rows at 500. Equal/sparse 50k-line documents can retain equality/context rather than being forced into two complete changed documents. Disjoint documents mount at most 500 rows per page, and omitted changes plus exact full sources remain reachable.

The repair bounds expensive diff search and mounted row count; it does **not** make complete diff memory constant. Input splitting, keys, operation streams and row metadata still grow O(L+R), with anchor selection O(K log K), where L/R are source line counts and K is unique common anchors. Complete Design Hall content can approach its existing 25 MiB per-version cap. These are explicit upper-workload acceptance limits; no observed freeze or leak is asserted.

## Execution evidence credited

VERIFICATION records Product/DiffView/find lifecycle controls **34/34**, including cache, paging, repeated-line reconstruction, cancellation and full-source access; actual Product HTTP controls **24/24**, including thin summaries over 100 large bodies, stable cursor ties/imports, older Unicode matches and authorization; and the subsequent **106-pass** state/git/product consumer batch after the `octet_length` projection repair. Controlled transport and template-input assertions establish behavioral bounds, not real DOM frame time or RSS.

Current merged evidence supplied by root: UI check 0 errors/0 warnings, 1333 unit passes, production build and bundle budgets green, 15 distinct desktop cases green across runs plus the API dirty-leave journey; publication desktop/phone light/dark captures inspected. All four current correctness issues are fixed; scheduler 57/57 is green. Historical broad Rust run was 4694 pass/2 fail/86 skip; the two failed checks subsequently passed individually. Remaining clippy/doc/build integration work is queued. These results are credited within their stated scope and do not measure large-content performance.

## Fixed-rubric provisional scores

| PLAN dimension | /2 | Evidence and concrete deduction |
|---|---:|---|
| Query/network work | 2.0 | Thin indexed keyset projection, lazy detail reads and explicit paged search; real HTTP regression evidence covers the repaired requests. No material gap in this bounded query-shape assessment. |
| Algorithmic/serialization cost | 1.9 | Budgeted diff search and 50k-line/repeated-line reconstruction controls. Deduct 0.1: the stated maximum-size artifact matrix still lacks long-single-line and near-25-MiB JSON comparison cost acceptance. |
| Retained memory/lifecycle | 1.8 | Four-body/4-MiB cache, cancellation/ownership controls and 500-row mounted bound. Deduct 0.2: no measured post-close recovery for repeated large-body expansion and full O(L+R) diff metadata at the supported artifact ceiling. |
| Scheduling/responsiveness | 1.8 | Bounded diff search, bounded mounts and yielding streamed search. Deduct 0.2: actual Product/Design Hall large-content long-task and interaction-latency acceptance under N=1/3/5 session load is absent, including native WebKit. |
| Representative CPU/RAM/latency measurements | 1.0 | Verified historical session baseline exists without these journey mappings. Deduct 1.0: no current content-workload CPU/RAM/latency series at N=0/1/3/5, sustained memory trajectory, or read-only real-app comparison. |
| **Total** | **8.5/10** | **No automatic 1.9 cap; the query dimension earns 2.0. Runtime evidence gaps prevent 9.8 acceptance.** |

## Remaining bounded acceptance

Root must measure the repaired transcript matrix (1/20/100 × 256 KiB, page/expand/find/cancel/close) and identical/sparse/disjoint/empty-side comparisons at 2,000/2,001/10,000/50,000 lines, plus a near-cap JSON and long-single-line case. Record response bytes, daemon/renderer CPU, heap/RSS, interaction latency/long tasks and mounted nodes. Run the isolated N=0/1/3/5 concurrent-session matrix and sustained open/close workload with comparable GC checkpoints; retain native WebKit and read-only real-app sampling separately. Existing Browser cache, Canvas history, Vault graph-degree and Snip pixel-workload caveats remain unmeasured, not newly promoted findings. No whole-application performance score or acceptance is inferred from this partition judgment.
