<script lang="ts">
  // The PR header's review pair, forge-style: Approve and Request changes,
  // each turning into a status ("Approved" / "Changes requested") once that is
  // your current review; the other button still switches opinion. A status is
  // itself a button: its menu withdraws that review (the parent confirms).
  // GitLab has no change request to withdraw, so that entry is left out there.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import type { MyReview } from './pr-review-state';

  interface Props {
    review: MyReview;
    busy: string;
    onapprove: () => void;
    onrequestchanges: () => void;
    /** Withdraw your current review (absent = statuses stay plain). */
    onwithdraw?: (kind: Exclude<MyReview, null>) => void;
    /** False where the forge can't withdraw a change request (GitLab). */
    canWithdrawChanges?: boolean;
  }
  let { review, busy, onapprove, onrequestchanges, onwithdraw, canWithdrawChanges = true }: Props = $props();

  function statusMenu(e: MouseEvent, kind: Exclude<MyReview, null>): void {
    if (!onwithdraw) return;
    ctxMenu.show(e, [
      kind === 'approved'
        ? { label: 'Withdraw approval…', icon: 'x', disabled: busy !== '', action: () => onwithdraw?.('approved') }
        : { label: 'Withdraw change request…', icon: 'x', disabled: busy !== '', action: () => onwithdraw?.('changes') },
    ]);
  }
  const menuFor = (kind: Exclude<MyReview, null>): boolean => !!onwithdraw && (kind === 'approved' || canWithdrawChanges);
</script>

{#if review === 'changes'}
  {#if menuFor('changes')}
    <button class="rv rv-changes rv-status" aria-haspopup="menu" disabled={busy !== ''} title="Your review: changes requested — click to withdraw"
      onclick={(e) => statusMenu(e, 'changes')}>
      <Icon name="warning" size={12} /> {busy === 'withdraw' ? 'Withdrawing…' : 'Changes requested'} <Icon name="chevronDown" size={12} />
    </button>
  {:else}
    <span class="rv rv-changes" title="Your review: changes requested">
      <Icon name="warning" size={12} /> Changes requested
    </span>
  {/if}
{:else}
  <button class="btn small rv-btn rv-request" disabled={busy !== ''} onclick={onrequestchanges} title="Request changes on this pull request">
    <Icon name="warning" size={12} /> {busy === 'request-changes' ? 'Requesting…' : 'Request changes'}
  </button>
{/if}
{#if review === 'approved'}
  {#if menuFor('approved')}
    <button class="rv rv-approved rv-status" aria-haspopup="menu" disabled={busy !== ''} title="Your review: approved — click to withdraw"
      onclick={(e) => statusMenu(e, 'approved')}>
      <Icon name="check" size={12} /> {busy === 'withdraw' ? 'Withdrawing…' : 'Approved'} <Icon name="chevronDown" size={12} />
    </button>
  {:else}
    <span class="rv rv-approved" title="Your review: approved">
      <Icon name="check" size={12} /> Approved
    </span>
  {/if}
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
  .rv-status { cursor: pointer; font: inherit; font-size: var(--fs-s); font-weight: 600; }
  .rv-status:disabled { cursor: default; opacity: var(--disabled-opacity); }
  .rv-approved { background: var(--success-soft); color: var(--success); border-color: var(--success); }
  .rv-changes { background: var(--warning-soft); color: var(--warning); border-color: var(--warning); }
  .rv-approve { color: var(--success); border-color: var(--success); font-weight: 600; }
  .rv-approve:hover:not(:disabled) { background: var(--success-soft); }
  .rv-request { color: var(--warning); border-color: var(--warning); font-weight: 600; }
  .rv-request:hover:not(:disabled) { background: var(--warning-soft); }
  @media (max-width: 1024px) {
    .rv { height: 34px; }
  }
</style>
