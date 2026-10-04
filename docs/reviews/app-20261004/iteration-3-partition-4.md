# Iteration 3 — narrow partition 4 recheck

**Verdict: Approve for the reviewed repair.** Confirmed remaining findings in this narrow pass: blocker 0 · major 0 · minor 0. **C4-R2-02 is addressed in source.** This is a source-review verdict, not a claim that tests or browser acceptance passed.

Read `iteration-2-implementation-4.md`, the current `WorkItemDetail.svelte`, the nine added Mission Control cases in `ui/unit/orchestrationOwnership.test.ts`, the PATCH client/server response contract, and the two Swarm nested-if simplifications. No builds, tests, servers, browser sessions, source edits, or delegation were performed. This report is the only authored file.

## Repair trace

| Path | Source evidence and result |
|---|---|
| Approval refresh while editing | `WorkItemDetail.svelte:126–131` updates persisted detail but only seeds draft fields when `!editing`. Request approval and Approve/Deny still refresh their metadata (`:195`, `:216`) while the active draft and explicit edit baseline remain intact. The original deterministic loss path no longer overwrites goal/result/risk. |
| GET started before Edit | The decision to preserve the draft is made when the response lands, not only when it is requested. Entering Edit and typing while the GET is pending therefore prevents reseeding at `:131`. |
| Remote persisted-field changes during editing | `editBaseline` is independent of `detail` (`:59`, `:90`). Updating remote goal/risk no longer silently changes the baseline used to judge the draft. Explicit Cancel and a new Edit seed the latest persisted detail (`:75–88`). |
| Read ordering and A→B→A | `load` records item/workspace ownership and increments a view generation on an owner change (`:108–122`). Success, failure and loading completion all require both view and read ownership. An older request cannot install into a later visit to the same item. Destruction invalidates ownership (`:64`). |
| Save versus older reads | Save invalidates earlier reads before PATCH and again after the owned response (`:168`, `:172`). Its returned persisted fields merge into the full detail (`:173`); the API actually returns a `WorkItem`, consistent with `missionControlApi.patch` and `routes/workgraph.rs:157–175`. Existing approvals/edges are preserved by that merge until the subsequent full read. |
| Typing after Save submission | Save captures submitted fields at `:166`. On success it advances the edit baseline to the saved response, but leaves editing and newer fields intact unless the current draft still equals the submitted snapshot (`:174–176`). The subsequent GET preserves that active draft. Cancel then restores the persisted saved version. |
| Save failure | The catch does not change draft fields or baseline; the owner-qualified finally releases busy state. The user retains a dirty, retryable draft. |
| Approval callback ownership | Both mutation methods capture item/workspace/view, reject callbacks for a different visit before refreshing, and owner-guard busy/decision cleanup (`:187–222`). They cannot refresh or clear another item's operation state after navigation. |

The inspected Edit and Cancel bindings invoke the new draft helpers; dirty navigation still uses `isDirty` against the explicit baseline. No new functional defect was confirmed in these paths.

## Regression-source assessment

The nine new cases at `ui/unit/orchestrationOwnership.test.ts:152–205` extract and execute the production draft helpers, loader, Save, Request approval and decision methods. They cover three approval variants; a read started before Edit; remote updates and baseline/Cancel behavior; stale GET after Save; newer typing during Save; failed Save; and A→B→A ordering. Assertions check persisted metadata alongside preserved fields and dirty/editing state, so the original unconditional reseeding would fail the relevant cases.

The fixture starts with an already loaded item and runs extracted methods with deferred API responses. It does not mount a Svelte component or exercise DOM bindings, effect scheduling, `onDestroy`, focus, or actual network cancellation. Approval POSTs resolve immediately in this fixture, so late approval-mutation completion after navigation is source-traced rather than directly covered by those nine cases. No test execution or observed red/green result is claimed by this reviewer.

## Swarm simplifications

At `crates/otto-server/src/swarm_agent_run.rs:279` and `crates/otto-server/src/swarm_run.rs:802`, nested `if !dispatched { if !send_run_input { stop } }` becomes the short-circuit conjunction. Rust's `&&` preserves call order: the second input attempt runs only when dispatch was not observed, and failed checked input still stops the turn. The lifecycle gate and readiness-wait boundaries are unchanged. No semantic regression found in these two edits.

## Retained prior dispositions and limits

The original dispositions from `iteration-2-partition-4.md` remain unchanged: C4-01…06, UX4-01…03 and D4-01 are addressed on the reviewed source paths; all five P4 mechanisms are implemented with performance measurements still requiring central evidence. Integration C4-R2-01 remains addressed. External Mission Control fullscreen-focus wiring remains subject to browser/accessibility acceptance. C4-R2-02 now moves from remaining-major to addressed-in-source.

This pass did not rerun the broader orchestration/performance review or invalidate its previously stated limits. Parent retains ownership of test/build gates, concurrent CPU/RSS and memory-over-time measurements, real-app sampling, and light/dark/mobile/browser verification. Approval of this bounded repair does not claim exhaustive correctness for every module or measured whole-app performance.

## Closing gate note — batch capability route policy

Source-verified the closing `policy_coverage` repair: `crates/otto-server/src/policy.rs:401` adds only the exact route template `/access/{kind}/{id}/capabilities/batch` to the existing `matches!` list beside single-item capabilities. It introduces no `starts_with`, wildcard, or resource-family prefix exemption. `Exempt` here bypasses only the central feature-axis lookup; it does not remove handler authentication/authorization.

The batch handler (`routes/resource_access.rs:528`) extracts `CurrentAuthContext`, checks `page_access(..., "discover")`, loads the live policy, requires resource-level discover, and calculates every child operation through `decisions_batch`, including page/legacy capability ceilings and the same pure policy evaluator. It strips administrative rule IDs before returning self-view decisions. The single handler at `:600` uses the equivalent discover/page gates and delegates its child decisions to the same batch implementation. Classifying this read-only POST alongside single capabilities is therefore consistent with the existing authorization design. No new defect found in this exact classification; the root-owned gate rerun was not executed by this reviewer.
