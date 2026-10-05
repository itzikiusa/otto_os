# Iteration 5 correctness — partition 3

**Verdict: Approve with fixes; 9.8 acceptance blocked.** Counts: blocker 0, major 1, minor 0, nit 0.

Source checkpoint: `418e963a`, `/Users/itziklavon/claude_ade-review`. Bounded source-only review of repaired Vault, Canvas, Design Hall, Product, Browser and Snip paths plus merged navigation/publication interactions. Read AGENTS.md, correctness-review skill, PLAN.md, VERIFICATION.md, iteration-4-correctness-3.md and iteration-4-role3-ui-recheck.md. No tests, builds, benchmarks, servers, source edits or git mutations performed. This report is the only file written. The execution evidence below is inherited, not a current-merge execution claim.

## Confirmed finding

### R5-C3-01 — major: dismissed Jira publication reselects its old story on completion

**Location:** `ui/src/modules/product/PublishDialog.svelte:243`–244; dismissal at `:270`, parent callback at `ui/src/modules/product/OverviewTab.svelte:2178`; guarded store publication at `ui/src/lib/stores/product.svelte.ts:620`.

**Intended:** A completed publication belongs to its initiating story; changing selection or dismissing its dialog must not let that obsolete operation take over the current story. The store explicitly captures ownership before its publication request and checks it before updating detail.

**Confirmed by hand trace, not runtime:** Select clean story A, review its preview and submit Jira publication. Hold the POST unresolved. Press Escape: PublishDialog passes no `dismissable` override to Modal; Modal's default is true (`ui/src/lib/components/Modal.svelte:20`) and Escape calls `onclose` (`:112`–114). Overview clears `publishDialogMode`, unmounting the dialog. Select clean story B. Resolve A's publication successfully. `publishAsStory` correctly declines to replace B's detail because its captured owner no longer matches; it returns A's resulting detail after the workspace list refresh. The still-running dialog continuation then sees A differs from `product.selectedId` and calls `product.select(A)` at line 244. B is replaced by A despite the user having left that operation. The new Product selection-to-URL effect (`ProductPage.svelte:329`) also writes A back into the URL. No dirty confirmation is required to reproduce: B can be clean.

The same obsolete completion calls its parent `onclose` at line 257. If the user dismissed then reopened a publication dialog in the same Overview instance, that callback clears the new dialog's mode too. This is the same missing initiating-dialog lifetime fence, not a separate finding.

**Repair:** Capture the initiating dialog lifetime, workspace and story/selection generation before submission. After each await, allow selection or parent closure only while that operation still owns the displayed dialog and selection context. Invalidate lifetime on destruction; a successful background publication can retain its factual completion notification without navigating. Merely checking story ID fails A→B→A, and the store's existing publication guard does not guard this caller.

**Regression:** Defer actual dialog submit, dismiss, select B, resolve A; assert B remains selected and its URL remains B. Add dismiss/reopen before completion and A→B→A controls, plus an ordinary active-dialog completion control. Prefer a mounted test because default Modal dismissal and parent callback identity are material to this trace.

Finding sent immediately to root and coordinator before report completion.

## Inspected repairs and adjacent traces

- **Browser:** `ui/src/lib/stores/browser.svelte.ts:112`–140, `:203`–305, `:329`–370. Traced reader/live creation A→B and A→B→A: workspace generation prevents late list/selection publication. Traced reader navigation A while B becomes visible: navigation uses its request-local returned page, and per-tab sequence rejects superseded navigation. Active close invalidates page/annotation ownership before awaiting transport; old success/error cannot republish; background close leaves the active reader alone; failed active close restores only matching ownership. No remaining defect established in these repaired paths.
- **Design:** `crates/otto-design/src/service.rs:1229`–1320. Approve and metadata patch both acquire the canonical artifact lock before reading, so two independent patches read successive durable rows rather than the same stale snapshot. Prior independent importer/thumbnail traces and the recorded 128-pass Design run support the adjacent writer boundary; no Rust source changes were introduced by the design merge according to VERIFICATION. This pass did not re-audit blob GC.
- **Canvas:** `ui/src/modules/canvas/ExcalidrawCanvas.svelte:207`, `:270`–279, `:311`–323, `:361`–384; `ui/src/lib/stores/canvas.svelte.ts:195`–245; `ui/src/modules/canvas/CanvasPage.svelte:55`–106, `:141`. Source load and initial data use the same persisted background/grid defaults; serialization retains them; the change fingerprint includes background/grid size/grid enabled so grid-only edits no longer disappear. Store close invalidates pending open; open prefers a retained failed-save draft. Automatic selection waits for list/current/pending/error conditions and uses the picking flag after reactive dependencies. Back-to-list catches save failure, but close does not delete the store's retained draft, so that alone is not proof of lost data.
- **Vault:** `ui/src/modules/vault/VaultPage.svelte:170`–226. Traced first-visit index selection versus saved tabs/non-note mode: the automatic open skips persisted views. Leaving the module routes through `flushBeforeLeave`; intra-vault moves defer to the store's existing guarded transition. Prior backlink and switcher repairs remain supported by the recheck/ledger; the merged loading/Retry presentation needs mounted verification rather than a fresh defect assertion.
- **Product navigation and paging:** `ui/src/modules/product/ProductPage.svelte:54`–70, `:280`–363; `ui/src/lib/stores/product.svelte.ts:234`–319, `:464`–570, `:608`–631, `:1032`–1040; `ui/src/modules/product/OverviewTab.svelte:139`–140, `:2175`–2180. Auto-selection defers to an explicit routed story; routed selection calls the existing draft-leave guard, waits while `routePending` suppresses URL writeback, and only resets the tab after the selected ID matches. A rejected different-story selection falls back to the still-selected story's URL. Transcript pages publish only under current story/generation/request ownership; body cache completions also require story ownership and are bounded. Mounted rune scheduling of initial route versus workspace teardown was not executed or asserted broken.
- **Publication preview:** `ui/src/modules/product/PublishDialog.svelte:90`–110, `:225`–270. Each preview clears the previous reviewed identity and hashes the request-local selected version body before publishing under sequence/story checks. Submission requires the selected preview identity, and 409 clears reviewed content for a fresh review. The completion selection exception is the finding above. The previously reviewed workflow preview CAS is supported by the inherited real-engine 30-pass run; this pass checked identity field references in `crates/otto-state/src/workflows.rs:1204`–1249 and `ui/src/modules/workflows/WorkflowsPage.svelte:230`, `:1336`, not a new full engine audit.
- **Snip:** `ui/src/modules/snip/SnipEditor.svelte:260`–325. Copy serializes in-flight operations and distinguishes persisted upload success from clipboard failure. Leave drains current annotations and only treats explicit discard as approval after failed persistence. No changed interaction defect established here.

## Actual evidence and bounded missing acceptance

Inherited ledger: premerge UI check **0 errors / 0 warnings**, **1314 unit passes**, all **12 distinct desktop acceptance cases** with passing executions after two assertion fixes, and **1 WebKit recap identity pass**. Named partition evidence includes Browser/Vault focused **18/18**, Vault combined **26/26**, Design **128 passed / 1 ignored**, and server **30/30** including exact Product publication preview/retry/approval. Read reports/ledger, not rerun by this reviewer.

Full affected gate: rustfmt and strict clippy passed; nextest **4694 passed / 2 failed / 86 skipped**. The two failures have source repairs with reruns pending. That run stopped before doc-tests/UI; neither full green nor current merged execution is claimed. The recap pass is not coverage for this partition's dialog lifecycle.

Bounded follow-up to acceptance:

1. Reproduce and repair R5-C3-01, then run its dismissed/reopened/selection-generation controls and ordinary completion control.
2. On the merged UI, mount direct Product URLs with automatic selection, cancel a dirty-story route change, and exercise initial mount/workspace selection ordering. Verify the retained draft and final URL, not only a store method's return.
3. Mount Vault failed-save leave/backlink Retry and Canvas restored background/grid → edit/save, plus retained failed-save draft after back-to-list. These are the specific boundary/recovery evidence gaps used below.
4. Run current merged UI checks/affected suites and rerun the two repaired Rust failures, then complete the interrupted doc-test/UI gate phases. Existing premerge passes remain valid historical evidence, not merge acceptance.

No native CDP, clipboard, renderer or broad application discovery was attempted; those unrelated omissions impose no additional numerical deductions here.

## Fixed PLAN dimensions

| Correctness dimension | Score /2 | Evidence and specific remaining deduction |
|---|---:|---|
| Contract/data integrity | 2.0 | Canonical metadata/approval serialization inspected; inherited Design 128-pass and Product exact-publication 30-pass evidence cover the repaired backend contracts, unchanged by the design merge. |
| State/concurrency ownership | 1.5 | R5-C3-01 is a proven stale dialog continuation overriding current selection; repair plus the mounted dismissal/reopen/ABA regression is required. |
| Boundary/error behavior | 1.9 | Existing failure guards and focused passes stand; merged Product dirty-route cancellation and Vault Retry/failed-save leave are the specific remaining mounted checks. |
| Persistence/recovery | 1.9 | Source normalization, fingerprint and retained-draft traces plus inherited repair regressions; merged Canvas restore→edit/save and failed-save back/reopen need mounted confirmation. |
| Executed regression coverage | 1.5 | Named passes cover substantial repaired matrix, but new dialog defect and the merged navigation/recovery matrix above are unexecuted; full affected gate remains failed pending reruns. |
| **Total** | **8.8 /10** | **Provisional bounded review with inherited execution evidence; not 9.8 acceptance.** |

Shell released: no child processes, tests, builds or background commands were started. No further shell work is pending for this reviewer.
