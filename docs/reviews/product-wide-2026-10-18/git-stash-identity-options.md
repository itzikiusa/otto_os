# Explicit stash Drop — identity-safe implementation options

Current confirmed defect: `LocalGit::stash_drop` resolves a requested SHA to `stash@{N}` before Git takes its ref lock. An external push/drop can renumber that selector. The app mutex cannot protect linked worktrees or external Git processes. Explicit Drop remains enabled; do not claim this defect is closed.

## Preferred candidate for independent review: native libgit2 transaction

Use the `git2` binding with minimal features rather than writing Git's ref files ourselves. No dependency or implementation has been added yet.

Proposed algorithm, on a blocking worker:

1. Open the repository with discovery disabled beyond the registered repository; reject unsupported ref/object formats before mutation. Resolve linked worktrees through the native library.
2. Create a transaction and `lock_ref("refs/stash")`. Lock failure becomes a recoverable conflict. Read the reflog **only after** taking the lock.
3. Find the requested full object ID among the reflog entries. If absent, release the transaction without mutations and return a stale-selection conflict. For duplicate identical object IDs, remove only the newest matching entry, matching the current selector behavior.
4. Remove the entry in memory with history rewriting enabled. Queue the resulting reflog through `set_reflog`. If its newest object changed, queue `set_target` to that object. If no entries remain, queue native removal of `refs/stash`; confirm the library also removes its empty reflog.
5. Commit, then release the native transaction. Surface every failure. Do not fall back to positional shell Drop if native handling refuses the repository.

`set_reflog` requires the reference to be locked and suppresses an extra automatic reflog entry when combined with target replacement. [libgit2 API](https://libgit2.org/docs/reference/main/transaction/git_transaction_set_reflog.html)

The Rust binding holds loose-reference locks through transaction lifetime, but **does not promise crash atomicity** between reflog and ref replacement: a reflog can be written even if updating the target subsequently fails. This candidate addresses the concurrency race; its failure recovery still requires review against Git's existing behavior. [git2 Transaction documentation](https://docs.rs/git2/latest/git2/struct.Transaction.html)

History rewriting changes the adjacent predecessor object ID so removing a middle entry does not leave a broken chain. [libgit2 reflog removal API](https://libgit2.org/docs/reference/main/reflog/git_reflog_drop.html)

Required independent checks:

- Demonstrate that system Git stash push/drop waits or refuses while the native transaction holds the common repository's stash ref lock, including callers from two linked worktrees.
- Top, middle, oldest, last and duplicate-SHA entries; preserve every remaining entry's object, message and order. Compare `git stash list`, `git reflog show` and actual recoverable contents with native Git results.
- Packed `refs/stash`, absent reflog, stale requested object and a concurrent branch/worktree registration; no fallback to mutable indices.
- Explicit SHA-256/reftable handling: support with verified native backend or refuse before mutation with an actionable message. Never reinterpret object IDs or unsupported storage.
- Inject write/rename/commit failures and process interruption. Identify exactly which recovery reference/reflog state remains; no false success. Compare with system Git's durability behavior.
- Bundled native dependency build/platform implications and repository config safety. Avoid enabling network/credential functionality just for local reflog mutation.

## Other alternatives

- Add an expected-object constraint inside Git's own reflog deletion implementation, checked after its ref lock is held. This would keep Git's backend/durability semantics, but needs a maintained Git/helper distribution or upstream support; no installed CLI primitive was found.
- Do not implement a handwritten reflog/ref writer: it duplicates backend, packed-ref, lock, SHA format and crash-recovery logic.
- An extra selector check, process-local mutex, advisory lock ignored by Git, or post-hoc wrong-entry recovery does not close the external-process race.

## Already implemented mitigation

Automatic restore uses `stash apply --index <full SHA>` and retains its recovery backup. Unique stash messages identify the captured automatic stash even if another stash is pushed before the subsequent read. This removes automatic wrong-entry deletion without changing explicit Drop's current availability. Pull/merge notes and the checkout UI copy explain retained backups.
