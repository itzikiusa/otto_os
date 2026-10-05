# Iteration 4 — current provisional partition 3 scores

Scope: Vault, Canvas, Design Hall, Product, Browser and Snip. Branch `fix/app-review-20261005`, current dirty repair checkpoint over baseline `03f2bc3e`; this is a bounded evidence synthesis, not a new discovery pass or final acceptance. The unchanged PLAN.md five-dimension rubric applies. All dimensions use 0.1 increments; none receives 2.0 with a material evidence gap.

**Current: correctness 8.9/10; performance 8.1/10; UX 8.5/10.** Confidence is moderate overall, high for the explicitly reproduced/repaired paths and lower for mounted/native behavior and performance. No 9.8 acceptance is claimed.

## Evidence used and original scores preserved

Read the original `iteration-4-correctness-3.md`, `iteration-4-performance-3.md`, `iteration-4-ux-3.md`, implementation/recheck reports and current `VERIFICATION.md`. Original scores remain **6.2 correctness, 7.0 performance, 7.4 UX**. The initial correctness execution dimension was0.0 because that reviewer ran no tests; under the later explicit PLAN calibration, the verified green baseline merits1.0, giving a separately calibrated initial correctness **7.2**. That calibration does not certify any new repair and does not overwrite the original report.

Current named evidence comes from root's actual execution, not this reviewer's own run:

- Browser/Canvas/Vault/destination batch39/39; subsequent browser close/lookup18/18; Vault26/26; latest Canvas11/11 including grid-only autosave and viewport-only negative control. These invoke production store/component handlers through controlled adapters, without Svelte mounting or native browser operation.
- Product transcript/find/shared-diff34/34; earlier expanded diff/cache22/22 includes reconstruction, pagination reachability and download-byte controls. These establish algorithm/handler bounds, not measured DOM/heap or native responsiveness.
- Product HTTP24/24: reviewed-content conflicts before external send, unchanged payload controls, thin paged summaries, cursor stability, Unicode search/detail and owning-story authorization. The later `octet_length` projection adjustment still needs its own final bundled-engine/consumer rerun; the separate Python EXPLAIN is not Rust runtime performance proof.
- Publication/recovery/destination UI17/17; workflow preview UI8/8. Actual server `review4_`30/30 now covers real-engine/API same-run stale approval rejection, current exact Jira/Confluence payload, ordinary approvals and migrated template. These use isolated outbound fixtures, not real external systems.
- Root's latest complete Design library result is128 passed/1 ignored, including original metadata and adjacent importer/thumbnail regressions. This supersedes the earlier125/3/1 checkpoint.
- A combined UI typecheck previously passed guards, Svelte0errors/0warnings and TypeScript, before subsequent backend contracts and Claude integration. Current combined consumer gates and mounted journeys are pending.

Counts overlap and must not be summed into a purported unique test total. VERIFICATION.md and the chronological recheck report retain RED/GREEN details.

## Correctness — 8.9/10

| Fixed dimension | /2 | Evidence and remaining deduction |
|---|---:|---|
| Contract/data integrity | 1.8 | Exact publication identity/body/destination has actual HTTP/engine coverage; metadata preservation now passes full Design library. Current final contract/type integration and the latest transcript byte-projection rerun remain pending. |
| State/concurrency ownership | 1.8 | Workspace/request generations, serial Browser mutations, close ownership, Vault lookup ownership and atomic approval CAS have source traces plus named regressions. Mounted rune/effect timing, native live tabs and broad cross-window permutations remain unverified. |
| Boundary/error behavior | 1.8 | Failed assist, failed close, lookup retry, stale publication409, missing approval identity and ordinary controls pass. The full native/clipboard/provider/error matrix is not executed. |
| Persistence/recovery | 1.8 | Design canonical locking passes original and adjacent tests; Canvas snapshot restore/grid autosave and prompt retention pass handlers. Real Excalidraw→backend reload and native/browser persistence journeys remain a bounded but material acceptance gap. |
| Executed regression coverage | 1.7 | Substantial current repair/failure coverage spans actual persistence, authorized HTTP and full workflow driver plus UI handlers. Above1.5 because major backend integration seams now execute; below1.8 because current final consumers, latest projection rerun and mounted/native paths are missing. |
| **Total** | **8.9** | **Provisional.** |

The original blocker/major triggers and independently discovered follow-ups are repaired with the evidence above. Do not carry the original bug deductions as though code were unchanged; equally, a passing fixture is not proof of the entire six-module matrix.

## Performance — 8.1/10

| Fixed dimension | /2 | Evidence and remaining deduction |
|---|---:|---|
| Query/network work | 1.8 | Product opens thin bounded pages and authorized bodies on demand; stable cursors and cross-history search pass HTTP tests. Latest byte-projection adjustment awaits bundled-engine revalidation and physical I/O measurement; legacy explicit full-body reads remain intentionally compatible. |
| Algorithmic/serialization cost | 1.8 | Diff equality/prefix/suffix/anchors and bounded expensive stages pass reconstruction cases;500-row pages preserve reachability. Total input/operation metadata remainsO(N+M), and huge single-line/JSON/native cost is not measured. |
| Retained memory/lifecycle | 1.8 | Four-body/4MiB cache, oversized download path and canceled/unregistered find/reveal ownership have passing production-handler coverage. Transient full downloads and diff metadata remain input-sized; sustained renderer/native memory recovery is unmeasured. |
| Scheduling/responsiveness | 1.7 | Bounded template rows, cancellation and backend offloading remove traced amplification. Meaningful gap: no current mounted Design compare/transcript latency or long-task evidence under representative load. |
| Representative CPU/RAM/latency measurements | 1.0 | Verified previous baseline concurrent/sustained session measurements are credited, but do not map to current large-content repairs. No current partition CPU/RAM/latency matrix, concurrent0/1/3/5 workload or sustained content/native sample. Test durations and SQL opcode evidence do not replace it. |
| **Total** | **8.1** | **Provisional; measurement gap remains substantial.** |

P3-01/P3-02 have concrete implemented bounds and regression evidence. This is an improvement judgment, not a measured whole-app speedup or a claim that count limits bound every transient byte allocation.

## UX — 8.5/10

| Fixed dimension | /2 | Evidence and remaining deduction |
|---|---:|---|
| Task completion/discovery | 1.8 | One-account destination retry, preserved Canvas prompts, full transcript find and complete diff navigation have named controls; template exposes an actionable configured preview→review→publish sequence. Mounted discoverability and actual provider creation remain pending. |
| Feedback/state clarity | 1.8 | Scoped loading/error/retry state, correct Browser titles and exact publication previews improve the traced paths. Tags/backlinks rendered states and integrated light/dark/accessibility feedback are not certified by handler tests. |
| Recovery/retry | 1.8 | Failed assist retains input, failed close restores content, lookup retries preserve fields, and publication409 demands another review. Complete mounted restore/retry journeys across formats/modules remain unexecuted. |
| Draft/scope/trust preservation | 1.8 | Frozen publication content and atomic displayed-identity approval now have real API/driver tests; draft and ownership handlers pass. Native close/clipboard and cross-window rendered behavior remain gaps; direct PublishDialog's optional account-base-URL binding is not equivalent to workflow's frozen URL check. |
| Executed end-to-end journeys | 1.3 | Above baseline1.0 because the real preview→human API rejection/retry→exact publication journey and full Design service flows now execute. Below1.5 because they omit the mounted human UI, and most of the six-module journey matrix remains handler-level/baseline only. No native or real external-system acceptance is claimed. |
| **Total** | **8.5** | **Provisional.** |

## Explicit remaining acceptance work

Current combined type/unit/affected Rust consumer gates; latest Product projection regression; mounted Browser navigation/close, Canvas assist/restore/save, Vault lookup/backlinks/recovery, Product publish/find and Design compare/restore; representative CPU/RAM/latency plus sustained memory under coordinated concurrent load; native WebKit/CDP, capture/clipboard and physical accessibility checks. Snip has prior verified baseline and bounded source inspection, not new current native coverage. Claude owns visual/accessibility integration; no design rescore is inferred here.

Only review documents were written. No tests, builds, servers, source changes, new discoveries or remote actions were performed for this rescore. Scores must be updated from the missing evidence rather than raised to a target by arithmetic.
