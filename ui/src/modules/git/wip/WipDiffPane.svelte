<script lang="ts">
  // The Changes view's per-file diff (WipPanel). Inline beside or under the
  // file list — or, on a phone, a pushed full-height sheet (DockedDrawer →
  // the shell Drawer: ✕, Esc and the system Back close it, lib/backCloses).
  import type { ComponentProps } from 'svelte';
  import type { DiffResp } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import DockedDrawer from '../../../lib/components/DockedDrawer.svelte';
  import DiffViewer from '../DiffViewer.svelte';
  import { splitPath } from './wipTree';

  interface Props {
    path: string;
    id: string;
    /** Staged and unstaged edits both exist: pick the side the hunk actions use. */
    both: boolean;
    stagedView: boolean;
    diff: DiffResp | null;
    loading: boolean;
    error: string | null;
    repoId: string;
    loadFile: ComponentProps<typeof DiffViewer>['loadFile'];
    wip: ComponentProps<typeof DiffViewer>['wip'];
    sheet: boolean;
    onretry: () => void;
    onclose: () => void;
  }
  let { path, id, both, stagedView = $bindable(), diff, loading, error, repoId, loadFile, wip, sheet, onretry, onclose }: Props = $props();
  const parts = $derived(splitPath(path));
</script>

{#snippet pane()}
  <div class="wp-diff" class:sheet {id}>
    <div class="wp-diff-head">
      <Icon name="file" size={12} />
      <span class="wp-diff-path" title={path} dir="ltr"><strong>{parts.name}</strong>{#if parts.dir}<span class="wp-diff-dir">{parts.dir}</span>{/if}</span>
      <span class="grow"></span>
      {#if both}
        <div class="segmented wp-target" role="group" aria-label="Diff side">
          <button class:active={!stagedView} aria-pressed={!stagedView} onclick={() => (stagedView = false)}>Unstaged</button>
          <button class:active={stagedView} aria-pressed={stagedView} onclick={() => (stagedView = true)}>Staged</button>
        </div>
      {/if}
      {#if !sheet}<button class="icon-btn wp-close" onclick={onclose} title="Close diff" aria-label="Close diff"><Icon name="x" size={14} /></button>{/if}
    </div>
    <div class="wp-diff-body">
      {#if loading && !diff}
        <div class="wp-diff-skel"><Skeleton rows={5} height={20} /></div>
      {:else if diff && diff.files.length > 0}
        <DiffViewer {diff} {repoId} {loadFile} {wip} />
      {:else if error}
        <LoadState variant="compact" what="this file’s diff" {error} empty {onretry} />
      {:else}
        <div class="wp-diff-empty">No textual diff for this file.</div>
      {/if}
    </div>
  </div>
{/snippet}

{#if sheet}
  <DockedDrawer open modal title={parts.name} {onclose} testid="wip-diff-sheet">{@render pane()}</DockedDrawer>
{:else}
  {@render pane()}
{/if}

<style>
  .wp-diff {
    flex: 1;
    min-height: 0;
    min-width: 0;
    display: flex;
    flex-direction: column;
    border-block-start: 1px solid var(--border);
  }
  /* The phone's sheet: the diff fills it (as PrAgentPanel's sheet does). */
  .wp-diff.sheet { flex: 1; border-block-start: none; }
  :global(.drawer-body > .dd-body:has(> .wp-diff.sheet)) { height: 100%; }
  .wp-diff-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 36px;
    padding-block: 4px;
    padding-inline: 12px 8px;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
    min-width: 0;
  }
  .wp-diff-head > :global(svg) {
    flex-shrink: 0;
    color: var(--text-dim);
  }
  .wp-diff-path {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
    overflow: hidden;
    white-space: nowrap;
    font-size: var(--fs-m);
    unicode-bidi: isolate;
  }
  .wp-diff-path > strong { font-weight: 600; flex-shrink: 0; }
  .wp-diff-dir { min-width: 0; overflow: hidden; text-overflow: ellipsis; font-size: var(--fs-s); color: var(--text-dim); }
  .wp-diff-body { flex: 1; min-height: 0; overflow-y: auto; overscroll-behavior: contain; }
  .wp-diff-skel { padding: 10px; }
  .wp-diff-empty { padding: 8px 14px; font-size: var(--fs-s); color: var(--text-dim); }
  .wp-target > button { height: 20px; padding: 0 6px; font-size: var(--fs-xs); }
  .wp-close { flex-shrink: 0; }
  .grow { flex: 1; }
  @media (max-width: 1024px) {
    .wp-diff:not(.sheet) { flex: none; height: min(70vh, 560px); }
  }
</style>
