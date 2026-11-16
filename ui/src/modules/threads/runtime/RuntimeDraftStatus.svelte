<script lang="ts">
  // The draft's save state, shown only when it failed (with Retry): "Saved" /
  // "Saving…" on every keystroke was noise beside the composer's controls.
  // Also why an attachment was not added (size / count, runtimeComposer.ts),
  // until it is dismissed or an attachment is removed.
  // The live region stays mounted so the failure is announced.
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { RuntimeComposerDraft } from '../../../lib/stores/runtimeDrafts.svelte';
  let { draft }: { draft: RuntimeComposerDraft } = $props();

  // Removing an attachment answers the refusal: the next one may fit now.
  let count = -1;
  $effect(() => {
    const n = draft.images.length;
    if (count >= 0 && n < count && draft.attachError) draft.attachError = null;
    count = n;
  });
</script>
<div class="draft-status" aria-live="polite" data-state={draft.saveStatus} data-testid="runtime-draft-status">
  {#if draft.attachError}
    <span class="attach-error" data-testid="runtime-attach-error">
      <Icon name="warning" size={12} />
      <span class="attach-text">{draft.attachError}</span>
      <button type="button" class="icon-btn" aria-label="Dismiss" title="Dismiss" onclick={() => (draft.attachError = null)}>
        <Icon name="x" size={12} />
      </button>
    </span>
  {/if}
  {#if draft.saveStatus === 'error'}
    <LoadState what="draft" errorTitle="Draft not saved" variant="compact" empty error={draft.saveError ?? 'Retry to save your draft.'} onretry={draft.retrySave} />
  {/if}
</div>
<style>
  .draft-status { font-size: var(--fs-xs); color: var(--text-dim); min-width: 0; }
  .draft-status:empty { display: none; }
  .attach-error { display: inline-flex; align-items: center; gap: 4px; min-width: 0; color: var(--warning); }
  .attach-text { min-width: 0; overflow-wrap: anywhere; }
</style>
