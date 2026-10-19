# Independent engineering review — explicit stash Drop, iteration 02

**Recommendation: implement a hybrid native transaction adapter, with a durable recovery record. Keep explicit Drop available.** Use libgit2 only for the locked, identity-selected reflog rewrite and nonempty-stack target update; use system Git's prepared `update-ref` transaction for final empty-stack deletion. Do not use libgit2 transaction removal, a positional CLI fallback, or a hand-written Git ref/reflog writer.

The original proposal is not ready unchanged. This review provides an implementable replacement, source evidence, and executed disposable fixtures. No Cargo command, dependency addition, or production source edit was performed. Workflow changes remain frozen. This is **not** a 9.8 verification claim: native-libgit2 execution and injected filesystem failures still need tests once the dependency is approved.

## Existing defect and independent findings

- **Major, confirmed source trace:** `crates/otto-git/src/local.rs:4117` resolves SHA to `stash@{N}` and only later invokes Git. An external push between those calls makes Drop delete a different entry. The comment on `resolve_stash_selector` incorrectly claims this gap is closed. An app mutex cannot synchronize external Git.
- **Major, confirmed source trace of the candidate library:** in libgit2 v1.9.7, `packed_delete` calls `packed_reload` before `packed_write` acquires the on-disk packed-refs lock. A packed-ref writer that commits between those calls can have its changes replaced from the earlier cache. This is a concrete interleaving, not an executed libgit2 fault injection. Avoid this path entirely for this feature; do not claim a loose-ref lock also protects unrelated packed refs. [Pinned native backend](https://github.com/libgit2/libgit2/blob/v1.9.7/src/libgit2/refdb_fs.c#L1412-L1512).
- **Major, confirmed source trace plus reproduced state:** libgit2 commits the custom reflog before the ref target. If target replacement fails, the stash list can show the remaining top entry while `stash@{0}` resolves to the removed object's stale ref target. The disposable probe reproduced exactly that state through native `git reflog delete --rewrite` without `--updateref`. The fix needs forward repair, not just an error return. [Native transaction implementation](https://github.com/libgit2/libgit2/blob/main/src/libgit2/transaction.c#L316-L348).
- **Minor, confirmed source trace:** transaction removal invokes `delete_tail` directly; unlike normal reference deletion, that path does not first remove the reflog. Do not assume `transaction.remove("refs/stash")` cleans up the empty log. The hybrid path lets system Git own both deletion operations. [Pinned removal implementation](https://github.com/libgit2/libgit2/blob/v1.9.7/src/libgit2/refdb_fs.c#L1242-L1257).

## Concrete adapter algorithm

Create `stash_identity` as a small local Git adapter; keep HTTP/UI and existing `stash_drop(sha)` behavior. Run blocking library operations off the async executor. The process mutex remains useful for Otto's own operations but is not the identity guarantee.

1. **Open and capability-check before mutation.** Resolve the registered repository, its actual common directory and storage/object formats using the hardened Git command. Open the same repository with native discovery disabled beyond the requested location. Initial supported set is `files` refs + SHA-1. Match the library's common directory to system Git's canonical result, including linked worktrees. Unknown format or disagreement returns a specific error before creating recovery refs or modifying anything. Do not accept a 64-character object ID through a 20-byte binding.
2. **Take the native stash ref lock, then read.** `transaction.lock_ref("refs/stash")`, then read its target and reflog through the native repository. Find the newest entry whose full object ID exactly equals the requested SHA. Empty/missing membership is a stale-selection conflict. Preserve the original target and all reflog entries, including duplicate-object occurrences. A symbolic `refs/stash` or an already inconsistent target/log is an explicit repair case, not an ordinary Drop.
3. **Protect recovery before destructive mutation.** While holding that stash lock, write a versioned app-owned journal containing the operation ID, canonical common directory, requested occurrence, original target, and exact pre/post reflog entries. Preserve message/committer bytes rather than lossy UTF-8 conversion. Pin every distinct old and new OID needed by either image under unique `refs/otto/stash-drop/<operation>/<n>` recovery refs using a **separate native Git ref transaction**; fully finish and verify it before modifying `refs/stash`. This is necessary because independent stash commits are not an ancestry chain. An incomplete journal/backup phase aborts Drop with the original stash untouched. Persist journal stages durably; this is application metadata, not a Git ref database writer.
4. **Rewrite exactly one occurrence in memory.** Use native reflog removal with rewrite enabled. For remaining entries, queue the new reflog and its newest OID with `set_target`, explicitly using the retained entry's signature so Drop does not suddenly require configured user.name/user.email; the explicit reflog suppresses an extra synthetic log entry. This writes only the loose stash target, including when it previously had a packed representation. For zero entries, queue only the empty reflog: **do not call native transaction removal**.
5. **Commit and classify actual outcome.** A returned error is not proof that nothing changed. Keep backups and journal until the observed ref/log state is classified. If the log equals the recorded postimage but its ref remains the recorded old target, reacquire the stash lock and forward-repair only after checking those exact conditions under the lock. If the state changed externally, do not restore the old snapshot over it. A coherent newer external state is preserved; an ambiguous state retains backups and produces an actionable recovery result.
6. **Finish the empty-stack case with system Git.** Launch hardened `git update-ref --stdin`, send `start`, `delete refs/stash <expected-old-oid>`, `prepare`, and wait for `prepare: ok`. Now, while that process holds the actual ref lock, reread the live reflog using native read APIs or a successful read-only Git command. Commit **only when the reflog is still empty**; otherwise abort. A missing/ref-changed prepare result means a newer external action may have won; re-read and classify without mutation. This protects an ABA sequence where another writer stores another stash and then the same original OID: checking only the target OID would delete both new entries. System Git performs packed-ref and reflog removal using its own backend. Never infer empty from a failed read. A timeout/cancellation closes or kills the owned child and waits for it; only its own lock lifecycle is involved.
7. **Verify and release recovery.** Report success only after confirming a coherent resulting stack or that a coherent newer external stack superseded cleanup. Return the selected occurrence's removal result, not a promise that its SHA can never appear again. On failure/interruption keep recovery refs and the journal; expose them to recovery rather than silently cleaning them up. Clean unique backup refs only after verified completion, with expected-OID deletion. Never auto-remove arbitrary `.lock` files.

The empty-log interval is deliberately journaled. It mirrors a failure state already possible in native Git and libgit2; it is not sold as crash-atomic. The protocol makes it detectable, forward-repairable, and preserves every remaining stash object. Power-loss durability additionally depends on the native backend/filesystem fsync behavior; it must not be equated with process-crash testing.

## Why locks work across tools/worktrees

The native files backend places shared refs in the common directory, and obtains the loose-reference lock before mutation. System Git's files reflog expiration takes the ref lock before reading/rewriting the reflog. The executable fixture held a system Git prepared transaction and showed both stash push and stash drop refusing from the main checkout and a linked worktree, with unchanged stacks and preserved working files. This verifies the system-Git side and shared path, **not an actual libgit2 FFI run**, since libgit2 is not installed here. [System Git files backend](https://github.com/git/git/blob/v2.46.0/refs/files-backend.c#L3170-L3296).

Git's `update-ref --stdin` exposes prepare/commit/abort, and deletions happen in transaction finish, so the live reflog can be inspected after prepare and before commit. [Transaction protocol](https://git-scm.com/docs/git-update-ref), [Git delete finish](https://github.com/git/git/blob/v2.46.0/refs/files-backend.c#L2890-L2995).

## Executed fixtures

Host Git: **2.46.0**. All repositories are disposable temporary directories; no user repo was mutated. Repro scripts and exact JSON observations are alongside this report:

- `git-independent-lock-probe.py` / `git-independent-lock-probe.json`: prepared ref lock; push/drop from main and linked worktree; log-written/ref-old mismatch; SHA-256 and reftable creation.
- `git-independent-cleanup-probe.py` / `git-independent-cleanup-probe.json`: ordinary and packed last-entry cleanup, different-OID arrival, and same-OID ABA arrival.

Observed cleanup results: ordinary and packed empty stacks commit successfully and remove `refs/stash`; the unrelated packed branch survives. An intervening different stash makes prepare refuse. An intervening two-store ABA sequence restores the original target OID but leaves two new reflog entries; the locked emptiness check aborts and preserves both. Recovery refs still resolve their stash commits in every case. A single `git stash store` of an unchanged current OID is a no-op on this Git version, so the test deliberately uses the two-store sequence rather than falsely claiming it produced an entry.

## Required implementation tests before acceptance

| Case | Required oracle |
|---|---|
| Top/middle/oldest/single | Exactly one intended occurrence removed; all other OIDs, messages, timestamps, identities and order preserved; ref equals newest remaining object or is absent |
| Duplicate OIDs | Newest matching occurrence only; preserve remaining duplicates and their separate messages |
| Native-library lock held | Actual external Git push/drop/clear/ref update from both main and linked checkouts refuses/waits; no working-tree change from failed push |
| External push/drop before lock | Select by current SHA under lock; no deletion if absent; never reuse a prior index |
| Packed stash + unrelated concurrent ref writes | Adapter never uses native packed deletion; all unrelated refs survive system-Git cleanup |
| Final-entry ABA arrival | Prepare can succeed on old OID, but nonempty log causes abort; both arrivals survive |
| Reflog write failure | Original log/target intact or classified; backups retained; no success |
| Target write failure after log commit | Exact postimage+old target is repaired under a fresh lock; newer external entries are never overwritten |
| Process death at every journal/commit boundary | Restart classifies before/post/partial/foreign states; every remaining stash remains reachable via original or recovery refs |
| Recovery plus aggressive GC (fixture only) | Every retained stash can still restore tracked, staged and untracked contents—not only `cat-file -t` |
| Empty/missing/corrupt reflog | Explicit classification; no fallback to dropping a numeric selector |
| Names/messages with invalid UTF-8, unusual signatures, missing user config | Round-trip bytes for retained entries; Drop uses retained signature instead of demanding new committer configuration |
| SHA-256/reftable | Pre-mutation unsupported error under the initial backend, no refs/journal mutations; future support requires the same identity/failure tests |
| API/UI | Explicit Drop stays enabled; conflict/failure surfaces selected identity and recovery guidance; no false success or hidden backup loss |

## Format support and dependency decision

System Git fixtures successfully created SHA-256 (64-character IDs) and reftable repositories. These cannot be assumed to work through the proposed default Rust binding: libgit2 describes SHA-256 as an experimental API-incompatible build, and the inspected stable extension allowlist does not include `refstorage`. Do not turn on an experimental ABI beneath an unmodified binding. [libgit2 build options](https://github.com/libgit2/libgit2/blob/v1.9.1/README.md#advanced-options), [extension handling](https://github.com/libgit2/libgit2/blob/v1.9.1/src/libgit2/repository.c#L1864-L1930).

Use an explicitly pinned git2/libgit2 combination with network defaults disabled, packaged consistently across macOS and CI; no dependency was added in this review. Keep Drop functional for supported repositories and return format-specific errors for unsupported repositories. If product acceptance requires Drop on SHA-256/reftable immediately, a **maintained helper built from Git's own backend code** with a locked expected-OID reflog-prune operation is the concrete alternative; it needs backend tests and packaging, not an unsafe positional fallback. That broader helper is unnecessary for the initial files/SHA-1 repair and should not be disguised as a small wrapper around the existing `git stash drop` CLI.

**Acceptance judgement:** the hybrid algorithm is a viable implementation direction. The present source is still uncorrected, and actual native-adapter/fault-injection tests are outstanding. A 9.8 rating is only supportable after those gates; source reasoning and the native-Git protocol probes alone do not justify it.

## Root regression implementation

Added `local::tests::stash_drop_keeps_sha_identity_when_stack_is_renumbered`: a Git shim returns the captured stash list, creates a second real stash, then lets production Drop continue. Assertions require the selected SHA to disappear and the new stash to remain restorable. Focused Rust execution is queued in the coordinated Cargo slot. A disposable system-Git reproduction of the exact current list/index/drop sequence ran and observed `selected_removed=false` and `concurrent_arrival_preserved=false`, confirming the wrong-entry deletion. No native adapter dependency has been added yet.

Rust baseline now executed: the new regression FAILED precisely at selected-SHA removal, as expected. Native adapter source and five focused tests are written in `crates/otto-git/src/stash_identity.rs`, but UNWIRED and NOT compiled. Revised singleton algorithm avoids the earlier empty-log phase: snapshot under native lock, release it, prepare system-Git conditional deletion, then compare the complete singleton while its lock is held before commit. ABA/nonempty arrivals abort and retry. Nonempty rewrites retain the journal/pins protocol because native reflog+target writes are not atomic. Ronaldo requested to independently review this implementation before integration.

## Implemented and verified — supersedes prototype status above

The adapter is now wired through `LocalGit::stash_drop` and pending recovery through `stash_list`. git2 0.21.0 is pinned with default network features disabled and vendored libgit2 enabled. The original deterministic wrong-entry regression failed before the change and passed afterward. Ten native regressions pass: selected stack positions, duplicate SHA, external Git locking including linked worktrees, singleton ABA, interrupted ref/reflog recovery, packed refs and missing user configuration, journal creation failure, stale selection, GC reachability, and intervening external push/clear.

Full Git library gate: **408 passed, 0 failed, 1 ignored** in 50.07s. All-target Git clippy with warnings denied passed in 14.28s. An old provider test was updated to assert Conflict for a stale repository snapshot and Invalid for the refreshed changed-forge snapshot; production destination validation was not relaxed.

Remaining: independent implementation review, server/review consumer gates, and final workspace integration. Fault fixtures model interrupted native writes; these are not real power-loss or disk-full experiments. Drop explicitly rejects unsupported repository formats instead of using positional deletion. No 9.8 score or release-readiness claim yet.

## Independent implementation review and consumer progress

Ronaldo independently traced native ref locking, singleton prepared-delete snapshot recheck, reachability pins and forward repair; no blocking defect found. An optional extra recovery call before locked rewrite was not added: it cannot close another external write window, while the locked mismatch path already refuses mutation and preserves recovery state for refresh/retry. Server test targets compile against typed credentials; full server library passed1087/0/1ignored. Final coherent workspace gates and post-extraction Git test build remain outstanding. Actual power-loss/disk-full testing is not claimed.
