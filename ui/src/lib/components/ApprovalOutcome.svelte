<script lang="ts">
  // The decided half of the approval shape (patterns §5): one line, "Approved
  // by Dana · 3m ago · “reason”". ApprovalActions swaps its buttons for this
  // once a request is decided; use it directly wherever only the outcome is
  // shown (a closed queue row, a decided card).
  import RelTime from './RelTime.svelte';

  interface Props {
    /** `approved` / `denied`, or `expired` for a request nobody decided. */
    outcome: 'approved' | 'denied' | 'expired';
    by?: string | null;
    at?: string | number | null;
    /** The reason / note recorded with the decision. */
    note?: string | null;
    testid?: string;
  }
  let { outcome, by, at, note, testid }: Props = $props();

  const word = $derived(outcome === 'approved' ? 'Approved' : outcome === 'expired' ? 'Expired' : 'Denied');
</script>

<div class="decided" data-testid={testid}>
  <span>{word}{by ? ` by ${by}` : ''}</span>
  {#if at != null && at !== ''}<span aria-hidden="true">·</span><RelTime iso={at} />{/if}
  {#if note}<span aria-hidden="true">·</span><span class="note">“{note}”</span>{/if}
</div>

<style>
  .decided {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 4px 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .note {
    overflow-wrap: anywhere;
  }
</style>
