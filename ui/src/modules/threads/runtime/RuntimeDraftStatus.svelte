<script lang="ts">
  // The draft's save state, shown only when it failed (with Retry): "Saved" /
  // "Saving…" on every keystroke was noise beside the composer's controls.
  // The live region stays mounted so the failure is announced.
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { RuntimeComposerDraft } from '../../../lib/stores/runtimeDrafts.svelte';
  let { draft }: { draft: RuntimeComposerDraft } = $props();
</script>
<div class="draft-status" aria-live="polite" data-state={draft.saveStatus} data-testid="runtime-draft-status">
  {#if draft.saveStatus === 'error'}
    <LoadState what="draft" errorTitle="Draft not saved" variant="compact" empty error={draft.saveError ?? 'Retry to save your draft.'} onretry={draft.retrySave} />
  {/if}
</div>
<style>
  .draft-status { font-size: var(--fs-xs); color: var(--text-dim); min-width: 0; }
  .draft-status:empty { display: none; }
</style>
