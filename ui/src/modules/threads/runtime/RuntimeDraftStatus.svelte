<script lang="ts">
  import type { RuntimeComposerDraft } from '../../../lib/stores/runtimeDrafts.svelte';
  let { draft }: { draft: RuntimeComposerDraft } = $props();
</script>
{#if draft.saveStatus === 'error'}
  <button type="button" class="draft-retry" onclick={draft.retrySave} title={draft.saveError ?? 'Draft not saved'}>Draft not saved · Retry</button>
{:else}
  <span class="draft-status" aria-live="polite">{draft.saveStatus === 'loading' ? 'Restoring…' : draft.saveStatus === 'saving' ? 'Saving…' : 'Saved'}</span>
{/if}
<style>
  .draft-status, .draft-retry { font-size: var(--fs-xs); color: var(--text-dim); white-space: nowrap; }
  .draft-retry { color: var(--warning); background: none; border: 0; cursor: pointer; }
</style>
