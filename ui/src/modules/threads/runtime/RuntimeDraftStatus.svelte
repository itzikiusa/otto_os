<script lang="ts">
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { RuntimeComposerDraft } from '../../../lib/stores/runtimeDrafts.svelte';
  let { draft }: { draft: RuntimeComposerDraft } = $props();
</script>
<div class="draft-status">
  <span aria-live="polite">{draft.saveStatus === 'error' ? '' : draft.saveStatus === 'loading' ? 'Restoring…' : draft.saveStatus === 'saving' ? 'Saving…' : 'Saved'}</span>
  {#if draft.saveStatus === 'error'}
    <LoadState what="draft" errorTitle="Draft not saved" variant="compact" empty error={draft.saveError ?? 'Retry to save your draft.'} onretry={draft.retrySave} />
  {/if}
</div>
<style>
  .draft-status { font-size: var(--fs-xs); color: var(--text-dim); min-width: 0; }
</style>
