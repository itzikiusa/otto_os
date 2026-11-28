<script lang="ts">
  // Merge-conflict resolver: a list / detail page (ListDetail). The list pane
  // "Conflicts" holds the conflicted files, each with its state as a word
  // (Conflict / Resolved), and the r of n progress in its footer; the content
  // is the ConflictFilePane of the selected file. On a phone the list is the
  // page and a file is pushed ("‹ Conflicts"). Under both: Abort (confirm)
  // and Complete (enabled once every file is resolved). Leaving the view
  // (abort/complete) is reported via `onleave`.
  import { onDestroy } from 'svelte';
  import type { GitOpInProgress, MergeConflictStatus } from '../../lib/api/types';
  import { toastError } from '../../lib/toastError';
  import { announceShown } from '../../lib/announce.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import SideRow from '../../lib/components/side/SideRow.svelte';
  import ConflictFilePane from './ConflictFilePane.svelte';
  import { gitPhoneMenu } from './gitPhoneMenu.svelte';
  import { canCompleteOperation } from './operationState';

  interface Props {
    repoId: string;
    /** Optional seed list so the view shows files immediately after a merge. */
    initialFiles?: string[];
    initialSource?: string | null;
    /** Which operation the conflicts belong to (merge / rebase / cherry-pick /
     *  revert); null for conflicts with no state file (a stash pop). Refined
     *  from the daemon's merge status once loaded. */
    initialOp?: GitOpInProgress | null;
    /** Called after the merge is completed or aborted (or there's nothing to do). */
    onleave: () => void;
  }
  let { repoId, initialFiles = [], initialSource = null, initialOp = null, onleave }: Props = $props();

  let status = $state<MergeConflictStatus | null>(null);
  let loading = $state(true);
  // Deliberately seeded from the prop's INITIAL value — refined from the
  // daemon's merge status as soon as it loads.
  // svelte-ignore state_referenced_locally
  let op = $state<GitOpInProgress | null>(initialOp);

  /** Human name of the operation, for titles/buttons ("merge", "rebase", …). */
  const opName = $derived(
    op === 'rebase' ? 'rebase'
    : op === 'cherry_pick' ? 'cherry-pick'
    : op === 'revert' ? 'revert'
    : op === 'merge' ? 'merge'
    : null,
  );

  // Files still conflicted (pulled from merge status). When a file is resolved
  // we move it from `pending` to `resolved` locally so the user sees progress
  // without a full reload.
  let pending = $state<string[]>([]);
  let resolved = $state<Set<string>>(new Set());
  let selected = $state<string | null>(null);

  let busy = $state<'' | 'abort' | 'complete'>('');

  // Phone: a picked file is pushed over the list (ListDetail ignores it elsewhere).
  let phoneOpen = $state(false);

  function selectFile(f: string): void {
    selected = f;
    phoneOpen = true;
  }
  const baseName = (p: string): string => p.split('/').pop() ?? p;
  const dirName = (p: string): string | undefined => (p.includes('/') ? p.slice(0, p.lastIndexOf('/')) : undefined);

  // A failed merge-status read with no seed to fall back on: shown with Retry
  // instead of a neutral "No conflicted files." while the repo is mid-merge.
  let loadError = $state<string | null>(null);
  let reloadKey = $state(0);
  let disposed = false;
  let statusGen = 0;
  onDestroy(() => {
    disposed = true;
    statusGen++;
  });

  $effect(() => {
    const id = repoId;
    void reloadKey;
    const gen = ++statusGen;
    let live = true;
    loading = true;
    loadError = null;
    void git
      .getMergeStatus(id)
      .then((s) => {
        if (!live || disposed || gen !== statusGen || id !== repoId) return;
        status = s;
        op = s.op ?? null;
        // Prefer the daemon's authoritative list; fall back to the seed.
        const files = s.conflicted_files;
        pending = [...files];
        resolved = new Set();
        selected = files[0] ?? null;
        // No merge in progress and nothing to resolve → bounce out.
        if (!s.merging && files.length === 0) {
          onleave();
        }
      })
      .catch((e) => {
        if (!live || disposed || gen !== statusGen || id !== repoId) return;
        loadError = loadErrorText(e);
        // Fall back to the seed list from the merge that brought us here.
        status = { merging: initialFiles.length > 0, source: initialSource, conflicted_files: initialFiles };
        pending = [...initialFiles];
        resolved = new Set();
        selected = initialFiles[0] ?? null;
      })
      .finally(() => {
        if (live && !disposed && gen === statusGen && id === repoId) loading = false;
      });
    return () => {
      live = false;
    };
  });

  const sourceLabel = $derived(status?.source ?? initialSource);
  // A/B side labels for the hunk pickers: A = the checked-out branch (ours),
  // B = the branch being merged in (theirs). Best-effort — plain OURS/THEIRS
  // when the repo status / merge source isn't known.
  const oursLabel = $derived(status?.ours ?? git.statusById[repoId]?.branch ?? 'OURS');
  const theirsLabel = $derived(sourceLabel ?? 'THEIRS');
  const allFiles = $derived([...pending]);
  const allResolved = $derived(canCompleteOperation(op, allFiles, resolved));

  function isResolved(path: string): boolean {
    return resolved.has(path);
  }

  function handleResolved(path: string): void {
    resolved = new Set(resolved).add(path);
    // Auto-advance to the next unresolved file for a smoother flow.
    const next = pending.find((f) => !resolved.has(f) && f !== path);
    // The last file: clear the pane — leaving it mounted invited a second
    // "Mark resolved" that 409'd (the file is no longer conflicted).
    selected = next ?? null;
    // Quietly reconcile against the daemon: a second pull / an agent / the CLI
    // may have added or resolved conflicts since this view seeded its list.
    void reconcile();
  }

  /** Re-read merge status and fold changes into the local lists: newly
   *  conflicted files are appended as pending; files no longer conflicted (and
   *  not resolved through this view) are marked resolved. Never removes rows —
   *  progress display stays stable. */
  async function reconcile(): Promise<void> {
    const id = repoId;
    const gen = statusGen;
    try {
      const s = await git.getMergeStatus(id);
      if (disposed || gen !== statusGen || id !== repoId) return;
      op = s.op ?? null;
      const live = new Set(s.conflicted_files);
      const known = new Set(pending);
      const added = s.conflicted_files.filter((f) => !known.has(f));
      if (added.length > 0) pending = [...pending, ...added];
      const externallyResolved = pending.filter((f) => !live.has(f) && !resolved.has(f));
      if (externallyResolved.length > 0) {
        const next = new Set(resolved);
        for (const f of externallyResolved) next.add(f);
        resolved = next;
      }
    } catch {
      /* best-effort — keep local state */
    }
  }

  async function abort(): Promise<void> {
    const label = opName ?? 'merge';
    const ok = await confirmer.ask(
      `Abort the ${label} and discard any resolutions made so far?`,
      { title: `Abort ${label}`, confirmLabel: `Abort ${label}`, danger: true }
    );
    if (!ok) return;
    busy = 'abort';
    try {
      const s = await git.abortMerge(repoId);
      if (git.primary?.id === repoId) git.primaryStatus = s;
      toasts.info(`${label[0].toUpperCase()}${label.slice(1)} aborted`);
      onleave();
    } catch (e) {
      toastError(`Couldn’t abort the ${label}`, e);
    } finally {
      busy = '';
    }
  }

  async function complete(): Promise<void> {
    if (!allResolved || busy) return;
    // No operation to conclude (stash-pop conflicts): the resolutions are
    // already written + staged — there's nothing to commit here, just leave.
    if (op === null) {
      onleave();
      return;
    }
    busy = 'complete';
    try {
      const result = await git.completeMerge(repoId);
      if (git.primary?.id === repoId) git.primaryStatus = result.repo_status;
      if (result.status === 'conflicts') {
        // Still conflicting (shouldn't normally happen) — refresh in place.
        pending = [...result.conflicted_files];
        resolved = new Set();
        selected = result.conflicted_files[0] ?? null;
        toasts.warn('Still conflicting', 'Some files still have conflicts.');
      } else if (result.repo_status.op_in_progress) {
        pending = [];
        resolved = new Set();
        selected = null;
        toasts.info('Operation paused', result.note ?? 'Amend the current commit, then continue.');
      } else {
        const done = opName ?? 'merge';
        toasts.success(`${done[0].toUpperCase()}${done.slice(1)} completed`, result.commit ? result.commit.slice(0, 8) : undefined);
        onleave();
      }
    } catch (e) {
      await reconcile();
      toastError(`Couldn’t complete the ${opName ?? 'merge'}`, e);
    } finally {
      busy = '';
    }
  }
</script>

<div class="resolver">
  {#if loading}
    <div style="padding: 16px; flex: 1"><Skeleton rows={6} height={28} /></div>
  {:else if loadError && allFiles.length === 0}
    <!-- The seed is empty, so there is nothing to fall back on: the repo
         may still be mid-merge, so this must not read as "nothing to do". -->
    <div class="load-fail">
      <LoadState what="the merge status" error={loadError} empty variant="page" onretry={() => reloadKey++} />
    </div>
  {:else}
    <ListDetail module="git" pane="conflicts" name="Conflicts" title={selected ? baseName(selected) : undefined} open={phoneOpen} onback={() => (phoneOpen = false)} empty={allFiles.length === 0}>
      {#snippet list()}
        <ListPane count={allFiles.length} menu={() => gitPhoneMenu.items()} testid="conflict-files">
          {#each allFiles as f (f)}
            <SideRow icon="file" title={baseName(f)} meta={dirName(f)} tip={f} selected={selected === f} testid="conflict-file" onclick={() => selectFile(f)}>
              {#snippet trailing()}
                <span class="file-state" class:resolved={isResolved(f)}>{isResolved(f) ? 'Resolved' : 'Conflict'}</span>
              {/snippet}
            </SideRow>
          {/each}
          {#snippet footer()}
            <span class="progress" class:done={allResolved}>{resolved.size} of {allFiles.length} resolved</span>
            {#if sourceLabel}
              <span class="ellipsis" title={sourceLabel}>· {opName === 'merge' ? 'merging' : opName === 'rebase' ? 'rebasing' : 'from'} <span class="mono">{sourceLabel}</span></span>
            {:else if op === null}
              <span class="ellipsis" title="From a stash pop or squash — resolutions are staged as you go">· from a stash pop</span>
            {/if}
          {/snippet}
        </ListPane>
      {/snippet}
      <div class="file-detail">
        {#if allFiles.length === 0}
          <EmptyState variant="page" icon="merge" title="No conflicted files" body="Nothing is left to resolve: finish or abort below." />
        {:else if selected && isResolved(selected)}
          <!-- Re-opening a resolved file: the daemon no longer serves it as a
               conflict (409), so a load error with a Retry that always fails
               was all this pane could show. -->
          <div class="detail-empty dim" use:announceShown={`${baseName(selected)} is resolved.`}>
            <Icon name="check" size={14} />
            <span>{baseName(selected)} is resolved.</span>
          </div>
        {:else if selected}
          {#key selected}
            <ConflictFilePane
              {repoId}
              path={selected}
              {oursLabel}
              {theirsLabel}
              onresolved={handleResolved}
            />
          {/key}
        {:else}
          <div class="detail-empty dim">
            <span>{allFiles.every(isResolved) ? 'All files resolved — continue the operation below.' : 'Select a file to resolve its conflicts.'}</span>
          </div>
        {/if}
      </div>
    </ListDetail>
  {/if}

  <footer class="resolver-foot">
    {#if op !== null}
      <button class="btn danger" disabled={busy !== ''} onclick={abort}>
        {busy === 'abort' ? 'Aborting…' : `Abort ${opName}`}
      </button>
    {/if}
    <span class="grow"></span>
    {#if !allResolved && allFiles.length > 0}
      <span class="dim foot-hint">Resolve every file to {op === null ? 'finish' : `complete the ${opName}`}.</span>
    {/if}
    <button class="btn primary" disabled={!allResolved || busy !== ''} onclick={complete}>
      {busy === 'complete' ? 'Completing…' : op === null ? 'Done' : `Complete ${opName}`}
    </button>
  </footer>
</div>

<style>
  .resolver {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    container-type: inline-size;
  }
  .grow {
    flex: 1;
  }
  .load-fail {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
  .file-state {
    color: var(--danger);
  }
  .file-state.resolved {
    color: var(--success);
  }
  .progress.done {
    color: var(--success);
    font-weight: 600;
  }
  .file-detail {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .detail-empty {
    display: grid;
    place-items: center;
    height: 100%;
    font-size: var(--fs-s);
  }

  .resolver-foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 10px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
  .foot-hint {
    font-size: var(--fs-xs);
  }
  /* Narrow: the hint takes its own line above the two buttons. */
  @container (max-width: 600px) {
    .foot-hint {
      flex-basis: 100%;
      order: -1;
    }
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
  @media (pointer: coarse) {
    .resolver-foot .btn {
      min-height: 40px;
    }
  }
</style>
