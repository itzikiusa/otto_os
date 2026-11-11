<script lang="ts">
  // The PR header's review pair, forge-style: Approve and Request changes,
  // each turning into a status ("Approved" / "Changes requested") once that is
  // your current review. Neither forge API we use can withdraw a review, so a
  // status is shown, not a toggle; the other button still switches opinion.
  import Icon from '../../lib/components/Icon.svelte';
  import type { MyReview } from './pr-review-state';

  interface Props {
    review: MyReview;
    busy: string;
    onapprove: () => void;
    onrequestchanges: () => void;
  }
  let { review, busy, onapprove, onrequestchanges }: Props = $props();
</script>

{#if review === 'changes'}
  <span class="rv rv-changes" title="Your review: changes requested">
    <Icon name="warning" size={12} /> Changes requested
  </span>
{:else}
  <button class="btn small rv-btn rv-request" disabled={busy !== ''} onclick={onrequestchanges} title="Request changes on this pull request">
    <Icon name="warning" size={12} /> {busy === 'request-changes' ? 'Requesting…' : 'Request changes'}
  </button>
{/if}
{#if review === 'approved'}
  <span class="rv rv-approved" title="Your review: approved">
    <Icon name="check" size={12} /> Approved
  </span>
{:else}
  <button class="btn small rv-btn rv-approve" disabled={busy !== ''} onclick={onapprove} title="Approve this pull request (asks first)">
    <Icon name="check" size={12} /> {busy === 'approve' ? 'Approving…' : 'Approve'}
  </button>
{/if}

<style>
  .rv {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding-inline: 10px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    font-size: var(--fs-s);
    font-weight: 600;
    white-space: nowrap;
  }
  .rv-approved { background: var(--success-soft); color: var(--success); border-color: var(--success); }
  .rv-changes { background: var(--warning-soft); color: var(--warning); border-color: var(--warning); }
  .rv-approve { color: var(--success); border-color: var(--success); font-weight: 600; }
  .rv-approve:hover:not(:disabled) { background: var(--success-soft); }
  .rv-request { color: var(--warning); border-color: var(--warning); font-weight: 600; }
  .rv-request:hover:not(:disabled) { background: var(--warning-soft); }
</style>
