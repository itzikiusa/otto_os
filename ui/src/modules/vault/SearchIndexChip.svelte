<script lang="ts" module>
  import { vaultSearchIndex } from '../../lib/api/vault';
  import { loadErrorText } from '../../lib/loadError';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { vault } from './vault.svelte';

  /** ⋯ → Rebuild search index: the index is derived, so this only re-reads
   *  the notes; search keeps answering (marked incomplete) meanwhile. */
  export async function rebuildSearchIndex(): Promise<void> {
    const v = vault.current;
    if (!v) return;
    const ok = await confirmer.ask(
      `Otto re-reads every note in “${v.name}” and rebuilds its full-text index in the background. ` +
        'Your notes are not changed. Until it finishes, search shows results from the part already indexed and says so.',
      { title: 'Rebuild search index?', confirmLabel: 'Rebuild' },
    );
    if (!ok || vault.current?.id !== v.id) return;
    try {
      const st = await vaultSearchIndex(vault.wsId, v.id, 'rebuild');
      if (vault.status && vault.current?.id === v.id && st.state !== 'ready') {
        vault.status = { ...vault.status, search_index: st };
      }
      toasts.info('Rebuilding the search index', 'Progress shows in the header; search keeps working.');
    } catch (e) {
      toasts.error('Couldn’t rebuild the search index', loadErrorText(e));
    }
  }
</script>

<script lang="ts">
  // Header chip while the vault's full-text index is incomplete (after an
  // upgrade or a rebuild), and the result toast when it completes.
  import { searchIndexChip, searchIndexDone, searchIndexNotice } from './searchIndex';

  const ix = $derived(vault.status?.search_index ?? null);
  // The vault whose rebuild this page watched, and its last known total.
  let watched: { id: number; total: number } | null = null;

  $effect(() => {
    const id = vault.status?.id ?? null;
    const cur = ix;
    if (cur && id !== null) {
      watched = { id, total: cur.total };
    } else if (!cur && watched && watched.id === id) {
      toasts.success('Search index rebuilt', searchIndexDone(watched.total));
      watched = null;
    } else if (watched && watched.id !== id) {
      watched = null;
    }
  });
</script>

{#if ix}
  <span class="ix-chip" title={searchIndexNotice(ix)}>
    {searchIndexChip(ix)}
  </span>
{/if}

<style>
  .ix-chip {
    display: inline-flex;
    align-items: center;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--info-soft);
    color: var(--info);
    font-size: var(--fs-xs);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
</style>
