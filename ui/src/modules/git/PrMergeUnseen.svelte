<script lang="ts">
  // The merge dialog's "you haven't read this" block: the PR's head moved
  // after its diff was loaded (pr-merge-gate.ts). Two ways on — reload the diff
  // and read the new commits, or tick the explicit acknowledgement.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import { unseenText, type UnseenHead } from './pr-merge-gate';

  interface Props {
    unseen: UnseenHead;
    /** "Merge including changes I have not reviewed" — bound by the dialog. */
    ack: boolean;
    disabled?: boolean;
    /** Close the dialog and re-read the PR + its diff. */
    onreload: () => void;
  }
  let { unseen, ack = $bindable(), disabled = false, onreload }: Props = $props();
  const headline = $derived(unseenText(unseen));
</script>

<div class="unseen" role="group" aria-label="Unreviewed changes" data-testid="pr-merge-unseen" use:announceShown={{ text: headline }}>
  <div class="unseen-head">
    <Icon name="warning" size={14} />
    <span class="unseen-title">{headline}</span>
  </div>
  <p class="unseen-text">Someone pushed after the diff you read was loaded. Merging now includes those changes.</p>
  <div class="unseen-actions">
    <button class="btn small unseen-reload" {disabled} onclick={onreload} title="Close this dialog and load the latest diff">
      <Icon name="refresh" size={12} /> Reload the diff
    </button>
  </div>
  <label class="unseen-ack">
    <input type="checkbox" bind:checked={ack} {disabled} />
    <span>Merge including changes I have not reviewed</span>
  </label>
</div>

<style>
  .unseen {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    border: 1px solid color-mix(in srgb, var(--warning) 45%, var(--border));
    background: var(--warning-soft);
    font-size: var(--fs-s);
  }
  .unseen-head {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--warning);
  }
  .unseen-title {
    font-weight: 600;
    color: var(--text);
  }
  .unseen-text {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .unseen-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .unseen-ack {
    display: flex;
    align-items: center;
    gap: 8px;
    cursor: pointer;
  }
  .unseen-ack input {
    accent-color: var(--accent);
    flex: none;
  }
  @media (max-width: 1024px) {
    .unseen-reload {
      height: 34px;
    }
    .unseen-ack input {
      width: 18px;
      height: 18px;
    }
  }
</style>
