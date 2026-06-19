# Batch 3 — S6: crash-safe / atomic auto-apply write

**File:** `crates/otto-improve/src/engine.rs`
**Scope:** Only the auto-apply WRITE in `process_edit`. The batch-2 injection
gate (`memory_content_gate`), the autonomy/risk decision (`classify::decide`),
and the queue semantics are unchanged.

## Problem

The auto-apply path (old `process_edit`, ~lines 359–373) was an unguarded
whole-file overwrite:

```rust
tokio::fs::write(&path, &edit.patch.after).await?;   // truncate-then-write
```

No conflict check, no atomic rename, no backup, no symlink defense. A crash
mid-write could leave a half-written `MEMORY.md` / `SKILL.md`; a concurrent edit
between the decision snapshot and the write was silently clobbered. (The
`approve`/`rollback` paths already conflict-check against `before_content`, but
auto-apply did not — and even they truncate-write in place.)

## Changes

All four are localized to the `Disposition::Apply` arm and two new free
functions (`safe_auto_apply`, `write_backup`) plus a `canonicalize_within`
helper and an `ApplyOutcome` enum. Nothing the DB stores changed
(`target_path` / `before_content` are still the resolved-but-not-canonicalized
path and the pre-decision snapshot, keeping approve/rollback consistent).

1. **Conflict check.** `safe_auto_apply` re-reads the current on-disk content
   immediately before writing and compares it to the `expected` snapshot the
   decision was based on (`current`, read at the top of `process_edit`). If they
   differ, it returns `ApplyOutcome::Conflict` and the file is left untouched —
   `process_edit` records the edit as `Pending` (queued for manual review)
   instead of `Applied`. This closes the TOCTOU window between decision and
   write, mirroring how `approve_edit`/`rollback_edit` guard against a changed
   file (they set `Conflict`; here, since the edit row doesn't exist yet, we
   queue).

2. **Atomic write.** New content is written to a hidden temp file in the *same
   directory* (`.<name>.otto-tmp-<nanos>`) and `rename()`d over the target —
   atomic on the same filesystem, so a crash can't leave a partial file. A
   failed rename cleans up the temp file and errors out.

3. **Backup.** Before overwriting (or before a `Remove` deletes), the previous
   content is written to a timestamped sibling `.<name>.bak-<UTC ts>` via
   `write_backup`, so an auto-applied edit (including a removal) is recoverable
   from disk, consistent with the rollback trail. Backups are only written when
   a prior file existed (a fresh-file create writes no backup), and never on a
   conflict (we never wrote).

4. **Canonicalize.** `canonicalize_within` canonicalizes the target (or, for a
   not-yet-existing file, its nearest existing ancestor + re-attached tail) and
   rejects any resolved path that escapes its allowed root — symlink
   defense-in-depth on top of the existing `pathsafe::resolve_target` segment
   guard. (`resolve_target` already blocks traversal/abs/`..`; this stops a
   symlinked leaf/parent from redirecting the write outside the dir.)

### Backup/temp files don't pollute prompts
The backup (`.*.bak-*`) and temp (`.*.otto-tmp-*`) files are hidden dot-files
and do not end in `.md`, so `read_memory` (which filters to `*.md`) never folds
them into future prompts.

## Behavior preservation
The normal happy path is unchanged in outcome: a clean low-risk edit still
auto-applies — now atomically, with a backup. The pre-existing
`memory_low_edit_applies_and_rolls_back` test (which exercises apply → rollback)
still passes through the new write path.

## Tests added (`engine::tests`)
- `auto_apply_writes_atomically_and_leaves_a_backup` — clean apply writes new
  content, leaves exactly one timestamped backup of the old content, and leaves
  no `.otto-tmp-` leftover (rename happened).
- `auto_apply_queues_on_conflict_instead_of_clobbering` — when the on-disk
  content differs from the snapshot, returns `Conflict`, the file is untouched,
  and no backup is written.
- `auto_apply_creates_new_file_when_snapshot_was_absent` — `None` snapshot +
  no file → clean create, no backup.
- `auto_apply_remove_backs_up_then_deletes` — a `Remove` backs the content up
  (recoverable) then deletes the file.

## Verification
- `cargo check -p otto-improve` — clean.
- `cargo clippy -p otto-improve --all-targets -- -D warnings` — exit 0, clean.
- `cargo test -p otto-improve` — 37 passed, 0 failed (4 new + all pre-existing).

No edits to files owned by other agents; no `cargo fmt`, no commit.
