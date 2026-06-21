<script lang="ts">
  // Merge-conflict resolver. LEFT: the conflicted files with a resolved /
  // unresolved indicator; RIGHT: the ConflictFilePane for the selected file.
  // Footer: Abort merge (confirm) and Complete merge (enabled once every file
  // is resolved). Leaving the view (abort/complete) is reported via `onleave`.
  import type { MergeConflictStatus } from '../../lib/api/types';
  import { git } from '../../lib/stores/git.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import ConflictFilePane from './ConflictFilePane.svelte';

  interface Props {
    repoId: string;
    /** Optional seed list so the view shows files immediately after a merge. */
    initialFiles?: string[];
    initialSource?: string | null;
    /** Called after the merge is completed or aborted (or there's nothing to do). */
    onleave: () => void;
  }
  let { repoId, initialFiles = [], initialSource = null, onleave }: Props = $props();

  let status = $state<MergeConflictStatus | null>(null);
  let loading = $state(true);

  // Files still conflicted (pulled from merge status). When a file is resolved
  // we move it from `pending` to `resolved` locally so the user sees progress
  // without a full reload.
  let pending = $state<string[]>([]);
  let resolved = $state<Set<string>>(new Set());
  let selected = $state<string | null>(null);

  let busy = $state<'' | 'abort' | 'complete'>('');

  $effect(() => {
    const id = repoId;
    loading = true;
    void git
      .getMergeStatus(id)
      .then((s) => {
        status = s;
        // Prefer the daemon's authoritative list; fall back to the seed.
        const files = s.conflicted_files.length > 0 ? s.conflicted_files : initialFiles;
        pending = [...files];
        resolved = new Set();
        selected = files[0] ?? null;
        // No merge in progress and nothing to resolve → bounce out.
        if (!s.merging && files.length === 0) {
          onleave();
        }
      })
      .catch(() => {
        // Fall back to the seed list from the merge that brought us here.
        status = { merging: initialFiles.length > 0, source: initialSource, conflicted_files: initialFiles };
        pending = [...initialFiles];
        resolved = new Set();
        selected = initialFiles[0] ?? null;
      })
      .finally(() => {
        loading = false;
      });
  });

  const sourceLabel = $derived(status?.source ?? initialSource);
  const allFiles = $derived([...pending]);
  const allResolved = $derived(allFiles.length > 0 && allFiles.every((f) => resolved.has(f)));

  function isResolved(path: string): boolean {
    return resolved.has(path);
  }

  function handleResolved(path: string): void {
    resolved = new Set(resolved).add(path);
    // Auto-advance to the next unresolved file for a smoother flow.
    const next = pending.find((f) => !resolved.has(f) && f !== path);
    if (next) selected = next;
  }

  async function abort(): Promise<void> {
    const ok = await confirmer.ask(
      'Abort the merge and discard any resolutions made so far?',
      { title: 'Abort merge', confirmLabel: 'Abort merge', danger: true }
    );
    if (!ok) return;
    busy = 'abort';
    try {
      const s = await git.abortMerge(repoId);
      if (git.primary?.id === repoId) git.primaryStatus = s;
      toasts.info('Merge aborted');
      onleave();
    } catch (e) {
      toasts.error('Abort failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }

  async function complete(): Promise<void> {
    if (!allResolved || busy) return;
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
      } else {
        toasts.success('Merge completed', result.commit ? result.commit.slice(0, 8) : undefined);
        onleave();
      }
    } catch (e) {
      toasts.error('Complete failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = '';
    }
  }
</script>

<div class="resolver">
  <header class="resolver-head">
    <Icon name="merge" size={14} />
    <span class="head-title">Resolve merge conflicts</span>
    {#if sourceLabel}
      <span class="head-source">merging <span class="mono chip">{sourceLabel}</span></span>
    {/if}
    <span class="grow"></span>
    {#if allFiles.length > 0}
      <span class="head-count" class:done={allResolved}>
        {resolved.size}/{allFiles.length} files resolved
      </span>
    {/if}
  </header>

  <div class="resolver-body">
    {#if loading}
      <div style="padding: 16px; flex: 1"><Skeleton rows={6} height={28} /></div>
    {:else}
      <!-- LEFT: conflicted files -->
      <aside class="files-panel">
        <div class="files-head">CONFLICTED FILES</div>
        {#if allFiles.length === 0}
          <div class="dim files-empty">No conflicted files.</div>
        {:else}
          {#each allFiles as f (f)}
            <button
              class="file-row"
              class:active={selected === f}
              class:is-resolved={isResolved(f)}
              onclick={() => (selected = f)}
              title={f}
            >
              <span class="file-status">
                {#if isResolved(f)}
                  <Icon name="check" size={12} />
                {:else}
                  <Icon name="dot" size={12} />
                {/if}
              </span>
              <span class="mono file-name">{f}</span>
            </button>
          {/each}
        {/if}
      </aside>

      <!-- RIGHT: selected file -->
      <div class="file-detail">
        {#if selected}
          {#key selected}
            <ConflictFilePane
              {repoId}
              path={selected}
              onresolved={() => handleResolved(selected!)}
            />
          {/key}
        {:else}
          <div class="detail-empty dim">
            <span>Select a file to resolve its conflicts.</span>
          </div>
        {/if}
      </div>
    {/if}
  </div>

  <footer class="resolver-foot">
    <button class="btn danger" disabled={busy !== ''} onclick={abort}>
      {busy === 'abort' ? 'Aborting…' : 'Abort merge'}
    </button>
    <span class="grow"></span>
    {#if !allResolved && allFiles.length > 0}
      <span class="dim foot-hint">Resolve every file to complete the merge.</span>
    {/if}
    <button class="btn primary" disabled={!allResolved || busy !== ''} onclick={complete}>
      {busy === 'complete' ? 'Completing…' : 'Complete merge'}
    </button>
  </footer>
</div>

<style>
  .resolver {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .resolver-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .head-title {
    font-size: 13px;
    font-weight: 600;
  }
  .head-source {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .chip {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 3px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .head-count {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--surface-2);
  }
  .head-count.done {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .grow {
    flex: 1;
  }

  .resolver-body {
    flex: 1;
    min-height: 0;
    display: flex;
  }
  .files-panel {
    width: 240px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
    padding: 6px 0;
  }
  .files-head {
    padding: 5px 12px;
    font-size: 10px;
    font-weight: 700;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .files-empty {
    padding: 6px 12px;
    font-size: 11.5px;
  }
  .file-row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 5px 12px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    text-align: start;
    transition: background 100ms, color 100ms;
  }
  .file-row:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .file-row.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    color: var(--text);
    box-shadow: inset 2px 0 0 0 var(--accent);
  }
  .file-row.is-resolved .file-status {
    color: var(--accent);
  }
  .file-status {
    flex-shrink: 0;
    display: inline-flex;
    color: var(--status-exited);
  }
  .file-name {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: rtl;
    text-align: left;
    font-size: 11.5px;
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
    font-size: 12px;
  }

  .resolver-foot {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 14px;
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
  .foot-hint {
    font-size: 11px;
  }
  .dim {
    color: var(--text-dim);
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
