<script lang="ts">
  // The header's quiet save state (the Hall autosaves): "Saved", "Saving…" or
  // "Couldn’t save · Save again" (a failed save, not a load, so not LoadState's
  // Retry). A status, not a call to action: named versions live in ⋯ and ⌘S.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import type { AutosaveState } from './autosave.svelte';
  import { saveIndicatorText } from './versionRows';

  interface Props {
    state: AutosaveState;
    dirty: boolean;
    error?: string | null;
    onretry: () => void;
  }
  let { state, dirty, error = null, onretry }: Props = $props();
  const text = $derived(saveIndicatorText(state, dirty));
  const failed = $derived(text === 'Couldn’t save');
</script>

<span class="save" class:failed data-testid="design-save-state" data-state={failed ? 'error' : text === 'Saved' ? 'saved' : 'saving'}>
  {#if failed}
    <span class="txt" title={error ?? 'Your edits are kept in this window.'} use:announceShown={{ text: `Couldn’t save. ${error ?? ''}`, assertive: true }}>
      <Icon name="warning" size={12} /> Couldn’t save
    </span>
    <button class="linkbtn" onclick={onretry} data-testid="design-save-retry">Save again</button>
  {:else if text === 'Saved'}
    <span class="txt"><Icon name="check" size={12} /> Saved</span>
  {:else}
    <!-- `design-dirty`: an edit not yet stored (kept for the e2e specs). -->
    <span class="txt" data-testid="design-dirty">Saving…</span>
  {/if}
</span>

<style>
  .save {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .txt {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .failed .txt {
    color: var(--danger);
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
  }
</style>
