# Iteration 6 correctness — partition 3

**Verdict: Approve the bounded repair; final acceptance pending.** New findings: blocker 0, major 0, minor 0, nit 0. **R5-C3-01 resolved** for the reported departure and reopened-dialog paths.

Runtime checkpoint: `64a850e6`; worktree `/Users/itziklavon/claude_ade-review`. Applied the correctness-review skill and unchanged PLAN rubric. Read current VERIFICATION.md, preserved iteration-5-correctness-3.md, and inspected only the publication ownership repair, its callers' ownership contract and mounted regression assertions. This is a source recheck with inherited actual execution evidence. No tests, builds, browsers, benchmarks, source edits, git mutations or agents were started. This report is the sole write.

## Repair disposition and hand traces

Inspected `ui/src/modules/product/PublishDialog.svelte:21`–23 and `:228`–282; `ui/src/lib/stores/product.svelte.ts:255`–278 and `:318`–327; `ui/e2e/desktop-review4-product-publication.spec.ts:146`–198.

- **Departed dialog:** Submit captures `product.captureSelection()` and combines it with component `alive`. Destruction clears `alive`. A's late Jira completion can report its successful publication, but returns before selecting A when the dialog is gone. It cannot call the parent close callback or set a stale conflict error. B remains selected; the URL writeback receives no obsolete selection.
- **A→B→A with a reopened dialog:** The old component remains dead even after another instance mounts. Independently, the captured store predicate includes workspace, story ID and selection generation; a different-story select increments that generation. Returning to A does not resurrect old ownership. The obsolete continuation therefore cannot close the new dialog. The mounted assertions verify both visibility/enabled action of the new dialog and final A URL.
- **Same-instance current operation:** With unchanged workspace/selection and a live dialog, the predicate stays true. The ordinary successful publication retains its success notification and close path. RFC completion also checks current ownership before parent closure. Failure handling returns before setting form/conflict state when ownership has departed; a current 409 still invalidates reviewed content and requests fresh review.
- **Store contract:** `captureSelection()` delegates to the same existing owner predicate used for asynchronous store writes; it does not change publication payloads or bypass draft-leave checks. `select()` still runs `mayLeaveDraft()` before a different selection and increments selection generation through `clearSelectionData()`.

The repaired caller now participates in the lifetime boundary already enforced by the store. No additional correctness defect was established within this bounded recheck. Prior Design, Browser, Canvas, Vault and Snip source dispositions remain as recorded in iteration 5; they were not broadly rediscovered or rescored on speculative omissions.

## Actual evidence, with provenance limits

Current VERIFICATION records the mounted departure test's intended RED (B lost selection after A returned), followed by GREEN after the caller lifetime/store-owner repair. Departure and History cases passed together **2/2**. The merged desktop run exercised ordinary Product content conflicts, destination recovery, departed publication and reopened A→B→A. Across the initial **14/15** and the corrected accessible-selector rerun **1/1**, all **15 distinct merged desktop cases passed**; this is not represented as one uninterrupted 15/15 run. Additional API Automation and reopened-publication capture passed **2/2**. The inspected Product test asserts retained selection/URL or retained reopened dialog after the delayed response actually finishes.

Merged/current UI evidence: **0 errors / 0 warnings**, **1333/1333 unit passes**, production build and unchanged bundle budgets passed. Runtime-checkpoint Rust formatting, strict all-target clippy, daemon build and workspace doc-test command passed; its 33 library targets each ran zero doc-tests, not 33 cases. Both earlier full-run failures now have actual focused GREEN reruns (route inventory and catalog byte budget, one case each). The original **4694 passed / 2 failed / 86 skipped** run remains historical; it has not become a retrospectively green full run.

Existing named backend evidence remains Design **128 passed / 1 ignored** and Product/workflow preview approval/retry **30/30**. No output from the pending acceptance queue is assumed. All execution above belongs to root's ledger, not this reviewer.

## Exact remaining acceptance work

1. **Canvas queued, unrun:** mounted failed, empty and accepted assistant prompts for all three formats. Carry forward the prior specific recovery checks: restored background/grid → edit/save and failed-save back/reopen preserves the retained draft, unless root can map an actual existing pass to those assertions.
2. **Vault queued, unrun:** failed-save leave and backlink failure → Retry, asserting retained note draft and current-note backlinks after recovery.
3. **Shared diff queued, unrun:** large-input paging and full download; this can supply Product's large-diff consumer evidence only when the executed fixture/assertions map to that path.
4. **Prior merged Product navigation acceptance:** departure/reopen tests now certify selection/URL ownership for R5-C3-01. They do not by themselves establish dirty-story route cancellation or first-mount URL selection versus workspace teardown. Retain those two named checks unless root supplies a matching mounted result.

These are concrete unexecuted acceptance cases, not asserted bugs. No broad native/CDP/clipboard/performance unknown imposes a further deduction. Final evidence rescore is pending root's actual queued outputs; an authored test is not a passing test.

## Fixed PLAN dimensions

| Correctness dimension | Score /2 | Evidence and bounded deduction |
|---|---:|---|
| Contract/data integrity | 2.0 | Existing inspected publication identity and Design metadata contracts retain named backend GREEN evidence; caller fix does not alter those contracts. |
| State/concurrency ownership | 2.0 | R5-C3-01's live/departed/reopened/ABA source predicates agree with actual mounted departure and reopen passes; no known defect remains in this repaired ownership matrix. |
| Boundary/error behavior | 1.9 | Publication conflicts/destination recovery passed; queued Vault failure/Retry, three-format Canvas rejected/empty prompts and prior dirty-route cancellation remain specific acceptance gaps. |
| Persistence/recovery | 1.9 | Prior normalized restore/retained-draft source and regression evidence stands; merged Canvas restore→edit/save and Vault failed-save recovery require mapped mounted results. |
| Executed regression coverage | 1.8 | Current affected repair, failure paths, UI gates and named merged journeys pass; queued Canvas/Vault/diff cases and the two prior navigation cases bound the remaining matrix. |
| **Total** | **9.6 /10** | **Provisional; final evidence rescore pending root outputs. Iteration 5 remains unchanged at 8.8.** |

**Shell released.** No background processes or further shell work remain with this reviewer.

## 2026-10-05 follow-up — initial Product URL repair and final evidence calibration

**Disposition: source repair accepted, mounted regression GREEN; no additional defect established.** The preceding 9.6 checkpoint remains preserved above. This append records later evidence and a final calibrated score rather than rewriting that checkpoint.

Root's new real initial-navigation case established RED in `/tmp/otto-review06-product-routing.log`: opening `#/product/A` with a saved workspace initially unavailable displayed newer story B. Inspected the uninstrumented repair in `ui/src/modules/product/ProductPage.svelte:282`–350 and the complete regression `ui/e2e/desktop-review6-product-routing.spec.ts:5`.

The workspace reset/load effect now precedes the selection effects. The route effect explicitly tracks `ws.currentId` and returns while it is absent; URL writeback also returns while the workspace is absent. Hand trace: initial null workspace leaves the explicit A URL intact; workspace arrival first clears prior selection and starts list loading, then retriggers the route effect, which records `routePending=A` before awaiting A's guarded selection. List auto-pick is suppressed during loading and subsequently defers to the explicit route. Writeback waits for the pending route and ultimately preserves selected A. This fixes the original missing reactive workspace dependency as well as reset ordering. No diagnostic reads that accidentally subscribe the effect to other state remain in the inspected block.

The regression then types an unsaved body, routes to B, chooses Keep editing, and asserts A's title, body and URL remain. A second attempt chooses Discard, asserts B/title/empty body/URL, and verifies A's persisted source stayed unchanged. I read `/tmp/otto-review06-product-green.log`: the uninstrumented case passed **three repeats, 3/3 in 6.6 seconds**. Root executed it; this reviewer did not. These results close both previously named initial-route and dirty-route acceptance gaps for the reported scenario, without certifying every possible workspace-switch interleaving.

Current VERIFICATION additionally records **18/18 mounted acceptance passes** and **7/7 recovery passes**. Relevant mappings: all three Canvas formats retain failed/empty prompts and clear accepted input; shared Diff exercises 50k/10k paging/full-source download; Vault backlinks failure/Retry, failed-open Retry/stale failure and failed-save/newer typing pass; Canvas large save/version restore, failed pending draft across scenes and Excalidraw failed hand-edit recovery pass. The environment cases in those totals are not counted as partition-3 coverage. These close the corresponding queued gaps recorded earlier.

One narrow evidence gap remains: the complete mounted **Excalidraw version restore of background/grid settings → subsequent hand edit → persisted settings** sequence. `desktop-canvas-versions.spec.ts:13` uses Mermaid. The three-format assistant recovery test includes a changed Excalidraw grid size and verifies accepted source persistence (`desktop-review5-canvas-assist-recovery.spec.ts:25`, `:46`), but it is not that exact restore-and-next-edit sequence. Existing inspected normalization/fingerprint code and focused regression evidence support the behavior; this is a minor acceptance gap, not an asserted defect or a request for a broad additional pass.

| Correctness dimension | Final score /2 | Final evidence and bounded deduction |
|---|---:|---|
| Contract/data integrity | 2.0 | Existing Design/publication backend passes retained; new routing repair changes no data contract. |
| State/concurrency ownership | 2.0 | Publication departure/reopen/ABA and initial route/workspace ordering have inspected repairs plus actual mounted passes. |
| Boundary/error behavior | 2.0 | Three-format rejected/empty/accepted prompts, Vault failed-save/Retry, Product Keep editing/Discard and existing publication conflict controls now have mapped passing executions. |
| Persistence/recovery | 1.9 | Mapped save/version/draft recovery passes close prior broad gaps; exact mounted Excalidraw settings restore→hand-edit persistence remains the single narrow gap above. |
| Executed regression coverage | 1.9 | Required bounded matrix now has named execution except that same specifically identified settings sequence; no authored/unrun case or retrospective full-suite GREEN is claimed. |
| **Final calibrated total** | **9.8 /10** | **Bounded partition acceptance with one minor evidence gap; not whole-application or native acceptance.** |

Historical scores remain R5 8.8 and initial R6 9.6. The initial Product URL defect was found and reproduced by root after the initial R6 report, and is now repaired and independently source-reviewed with actual GREEN evidence. No new reviewer tests/builds/browser sessions/source edits/git mutations occurred. **Shell released.**
