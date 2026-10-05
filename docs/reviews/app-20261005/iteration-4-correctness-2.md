# Iteration 4 — correctness partition 2

**Verdict: Block.** Confirmed by hand trace: 2 blocker, 3 major, 0 minor. No execution was performed in this review.

Source: branch `fix/app-review-20261005`, HEAD `03f2bc3e380cbee6da22723872ba865a84aa8050`, inspected 2026-10-05. Partition: Git/workbench, database explorer/connections, brokers, API client. Read repository AGENTS.md, this effort's PLAN.md and correctness-review skill/severity instructions. Historical partition-2 repairs are closed; these findings concern adjacent current behavior, not historical pending statuses. Only this report was written; no builds, tests, servers, source edits or commits.

## Ranked findings

### C4-2-01 — blocker: replay changes binary records and turns tombstones into empty values

- **Location:** `crates/otto-brokers/src/service.rs:1049`, `:1054`, `:1055`, `:1060`; producer sink `crates/otto-brokers/src/kafka.rs:1504`, `:1518`, `:1523`.
- **Intended:** `BrokerService::replay` reads raw records and republishes them, changing only the explicitly requested key/header transform. A binary key/value/header and an absent value must retain their meaning.
- **Concrete trace:** A source record has key bytes `[0xff]`, value bytes `[0xff, 0x00]`, and header `x-bin=[0xff]`, without a transform. The raw key fails `from_utf8(...).ok()` and becomes `None`. `from_utf8_lossy` converts value/header `0xff` into U+FFFD. Both base64 flags are false. `KafkaClient::produce` encodes these strings as UTF-8, publishing a null key, value `[0xef,0xbf,0xbd,0x00]` and header `[0xef,0xbf,0xbd]`. Successful production/evidence reports a replay despite changed bytes. Separately, source `value=None` becomes `Vec::new()` at line 1049 and the producer always calls `.payload(&value)`, producing a present zero-byte value instead of a tombstone.
- **Actual versus intended:** Binary event/schema payloads are corrupted, key-based partitioning/compaction semantics change, and replaying a deletion can instead create an empty record. This is persisted incorrect output on a supported broker path.
- **Confidence:** Confirmed by source hand trace; not executed against Kafka.
- **Repair:** Introduce a byte-preserving internal producer accepting optional key/payload bytes and raw header values; have replay call it directly. Apply text transforms only to the selected fields. Keep absent and empty payloads distinct. Existing HTTP text/base64 requests can adapt into the same producer; do not merely base64-fix the value while leaving headers or tombstones lossy.
- **Regression:** In an isolated broker fixture replay a binary-key/binary-value/binary-header record, a tombstone, and a present-empty record; consume raw target records and assert exact byte equality and `None` versus `Some([])`. Add pure transformation tests for key/header overrides. The existing produce contract also claims an empty non-base64 value creates a tombstone (`docs/features/message-brokers.md:405`, `docs/contracts/api.md:3126`), whereas this sink always sets a payload; explicitly reconcile and test that public contract when introducing the raw adapter.

### C4-2-02 — blocker: guarded-write confirmation names a different connection from the retry target

- **Location:** `ui/src/lib/stores/database.svelte.ts:3371`, `:3408`, `:3424`, `:3426`; ordinary query retry also calls the same helper at `:3241` and `:3256`.
- **Intended:** The typed confirmation at lines 3365–3368 authorizes a write to the actual named connection. Navigation must not change the identity being approved.
- **Concrete trace:** Start `runManagedStatement('DELETE FROM orders', 'db:app')` on production connection A. It captures A's ID at 3408 and submits `confirm_write:false`. Before that request's refusal arrives, select already-open connection B (the supported `openConnection` path updates `selectedConnId` at 2213). The delayed A refusal reaches line 3424. `confirmGuardedWrite` reads `this.selectedConn`, now B, and asks the user to type B's name. Typing B returns true. Line 3426 retries the captured A URL with `confirm_write:true`, executing on A with no confirmation that names A. The same helper is used by `runQuery`; switching ordinary connection tabs does not itself increment the access epoch.
- **Actual versus intended:** A user approves B while a destructive write runs on A. The captured request URL is correct; the approval target is wrong.
- **Confidence:** Confirmed by source hand trace, including connection-switch implementation; not executed.
- **Repair:** Capture the originating connection identity/name/environment and request ownership at run start. Pass that explicit target into the confirmation helper instead of reading current selection; either cancel a superseded operation or keep its confirmation and retry bound to A. Apply the rule to both managed statements and ordinary/agent query retries, retaining the existing access/cancel checks.
- **Regression:** Deferred-promise unit tests: submit against guarded A, switch to B before returning a write-blocked response, then verify either cancellation with no second POST or a prompt naming A whose accepted retry still targets A. Test `runManagedStatement` and `runQuery`, both cancellation and acceptance, plus access revocation before confirmation resolves. No real database write is necessary.

### C4-2-03 — major: Unicode key previews can panic the replay request

- **Location:** `crates/otto-brokers/src/service.rs:1019`.
- **Intended:** The best-effort evidence preview truncates long valid UTF-8 keys without preventing record replay.
- **Concrete trace:** The source key is 63 ASCII `a` characters followed by `é`. UTF-8 decoding succeeds and length is 65 bytes, so the `s.len() > 64` branch executes. `&s[..64]` ends between the two bytes of `é`, which panics because 64 is not a UTF-8 boundary. Production for that record and the final replay evidence insert are never reached; earlier records in the loop may already have been produced.
- **Actual versus intended:** A valid key interrupts replay instead of producing a shortened preview. This is a request-task panic, not a demonstrated whole-daemon crash.
- **Confidence:** Confirmed by Rust string-slicing semantics and hand trace; not executed.
- **Repair:** Use a character-safe bounded preview helper (`chars().take(...)` or an explicitly checked byte boundary); keep this evidence-only operation non-failing.
- **Regression:** Unit-test a 63-ASCII-plus-`é` key and multi-byte-only keys crossing the limit, then assert replay transformation can proceed and the preview remains valid UTF-8. Include empty, short, and exactly-at-boundary cases.

### C4-2-04 — major: API-client late completions publish workspace A state into workspace B

- **Location:** `ui/src/lib/stores/apiClient.svelte.ts:760` (`loadEnvironments`), `:1312` (`activateEnvironment`). Same unfenced publication pattern at `:736` (collections load), `:979` (collection save), `:1285` (environment save), `:1608` (automation load) and `:1625` (automation save).
- **Intended:** Lists and active environment describe the current workspace, as explicitly enforced by `loadAll`'s workspace check and `saveRequest`'s base check. An old workspace's operation must not mutate the new workspace's client state.
- **Concrete trace A:** A refresh starts `loadEnvironments()` in workspace A and captures A's base URL. Switch to B and let `loadAll()` finish installing B's environments. A's delayed GET then finishes and line 760 assigns A's whole list into the currently displayed B store without checking the base. B's environment picker now displays A data. `execute()` reads `activeEnv.id` at line 1354 while posting to B's base, so its request can carry A's environment ID.
- **Concrete trace B:** Start `activateEnvironment(a)` in A, switch to B, and let B's list load with active environment b. A's response then resolves; line 1312 maps over B's list with `e.id === a`. IDs differ, so all B environments become inactive locally although the B server state still has b active.
- **Actual versus intended:** Visible lists/active selection disagree with the current workspace and can supply a foreign environment ID to execution. This report does not claim the backend authorizes that foreign ID.
- **Confidence:** Confirmed by source hand trace. `loadAll` and request save already have ownership checks; these independent methods do not. No runtime execution.
- **Repair:** Bind each loader/mutation's publication, error and freshness state to its originating workspace and generation. Return a saved server object to the caller if needed without inserting it into another workspace. Audit the listed sibling methods together rather than fixing only the list loader. Preserve server-side completion in A.
- **Regression:** Deferred HTTP tests for refresh, save and activate: start in A, finish B's normal load, resolve A last, and assert B lists, active environment and cached freshness remain B's. Check the next B execute carries b, never a. Include a stale A error so it cannot erase B's successful load state.

### C4-2-05 — major: environment Save discards edits made while the request is pending

- **Location:** `ui/src/modules/api/EnvironmentsView.svelte:117`, `:119`; editable inputs at `:233` and `:241` remain enabled during Save.
- **Intended:** Save persists the submitted environment snapshot while subsequent edits remain an unsaved draft. A successful response must not erase input the user entered after submitting.
- **Concrete trace:** In one environment, set `base_url='first'` and click Save. The body built at lines 99–115 contains `first`. Before the PATCH returns, type `second` into the same field. Input remains enabled (`disabled={!canEdit}`), so `updateRow` changes the draft to `second` and sets dirty. The server returns the correctly persisted `first`; line 119 calls `seed(saved)`, which replaces rows with `first` and resets dirty=false (lines 42–45). `second` is lost and Save becomes disabled.
- **Actual versus intended:** A successful save silently destroys a newer unsaved edit. Selecting a different environment during the wait is also not guarded by save ownership, but the single-environment trace alone establishes this finding.
- **Confidence:** Confirmed by source hand trace; no mounted-component execution.
- **Repair:** Capture environment ID plus draft generation or submitted snapshot; reseed only if that same draft is unchanged. Otherwise preserve current rows/dirty state and adopt saved secret markers carefully only for unchanged members. Alternatively disable editing and environment switching for the complete save operation, if that interaction is intentionally chosen.
- **Regression:** Mount the environment editor with a deferred save; submit `first`, type `second`, resolve the response, and assert `second` remains visible and dirty (or that editing is intentionally disabled while pending). Follow with a second Save and assert it submits `second`. Include switching to another environment before completion to ensure its rows are not replaced.

## Provisional score

These are **source-review judgments**, not executed reliability measurements. The original provisional score remains here; root should append execution-adjusted evidence separately.

| Correctness dimension | /2 | Evidence and deduction |
|---|---:|---|
| contract/data integrity | 1.1 | Raw broker path demonstrably changes bytes/nullness; SQL target restrictions and Git hunk fingerprint checks inspected positively. |
| state/concurrency ownership | 1.1 | Database approval target and API workspace/save races confirmed; existing per-tab API send controllers and database query controllers inspected. |
| boundary/error behavior | 1.4 | UTF-8 boundary panics replay; guarded retry takes the wrong target metadata. Other inspected error/abort gates are explicit. |
| persistence/recovery | 1.6 | Newer environment draft is lost; replay success persists altered data. Git stash/index restoration and reviewed-change immutable claim checks have explicit recovery/binding mechanisms in source. |
| executed regression coverage | 1.2 | Prior effort has recorded scoped passes, and relevant existing regression source was inspected; none of the five new triggers was executed here. No Kafka/native-engine or browser result is inferred from source. |
| **Total** | **6.4/10** | **Provisional; blockers prevent acceptance independently of arithmetic.** |

## Coverage and limits

- **Git/workbench:** Inspected WIP partial-stage separation, staged/worktree diff choice, keyed repository mounting, deferred pull/push retries, and `run_hunk_op` request validation, fingerprint check, backup-before-discard and repo lock. No additional confirmed Git defect in these finite traces. Historical auto-stash preservation was read as a closed source disposition, not rerun.
- **Database/connections:** Traced query ownership, connection snapshots/switching, managed write gate, inline-edit review/refresh ownership, comparison read-only wiring, and connection form save/test scope. Inspected SQL editability guards and reviewed-change persisted attempt/script/hash/policy/credential binding with pre-execution revalidation. Did not run a native driver, approved-change execution, or complete multi-run/import/export workflows.
- **Brokers:** Traced replay HTTP authorization/guard through raw consume, transforms, producer sink and final evidence. Inspected schema version and tail repairs via the prior independent recheck and current adjacent files. No live Kafka, schema registry, failure-after-partial-production or consumer reset experiment was run; transactional/partial replay recovery remains an execution gap, not a separate asserted defect here.
- **API client:** Traced request snapshot and per-tab execution ownership, script variable write merging, draft save ownership, environment editor/store mutation, workspace list loading, collection import and automation loading. Historical automation leave guards remain closed. No external HTTP, OAuth, SSH tunnel, rendered UI or storage-quota journey was executed.
- **Not this lens:** No visual/a11y/copy judgment; Claude owns those. No CPU/RAM or performance score. Hand traces are explicitly distinguished from reproducible tests executed by root later.
