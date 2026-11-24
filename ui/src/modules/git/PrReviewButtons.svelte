<script lang="ts">
  // The PR header's ONE review control: "Review ▾" — Approve · Request
  // changes · Comment. It is the header's primary while reviewing is your next
  // step (prStatus.reviewPlacement), else the one visible secondary; once you
  // reviewed it names your verdict ("Approved ▾" / "Changes requested ▾") and
  // its menu can withdraw it (the parent confirms). GitLab has no change
  // request to withdraw, so that entry is left out there.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import type { MyReview } from './pr-review-state';

  interface Props {
    review: MyReview;
    busy: string;
    primary?: boolean;
    onapprove: () => void;
    onrequestchanges: () => void;
    oncomment: () => void;
    /** Withdraw your current review (absent = no withdraw entries). */
    onwithdraw?: (kind: Exclude<MyReview, null>) => void;
    /** False where the forge can't withdraw a change request (GitLab). */
    canWithdrawChanges?: boolean;
  }
  let { review, busy, primary = false, onapprove, onrequestchanges, oncomment, onwithdraw, canWithdrawChanges = true }: Props = $props();

  const label = $derived(
    busy === 'approve' ? 'Approving…'
    : busy === 'request-changes' ? 'Requesting…'
    : busy === 'withdraw' ? 'Withdrawing…'
    : review === 'approved' ? 'Approved'
    : review === 'changes' ? 'Changes requested'
    : 'Review',
  );

  function open(e: MouseEvent): void {
    const items: MenuItem[] = [];
    if (review !== 'approved') items.push({ label: 'Approve…', icon: 'check', disabled: busy !== '', action: onapprove });
    if (review !== 'changes') items.push({ label: 'Request changes…', icon: 'warning', disabled: busy !== '', action: onrequestchanges });
    items.push({ label: 'Comment', icon: 'comment', action: oncomment });
    if (onwithdraw && review === 'approved') items.push({ separator: true }, { label: 'Withdraw approval…', icon: 'x', disabled: busy !== '', action: () => onwithdraw?.('approved') });
    if (onwithdraw && review === 'changes' && canWithdrawChanges) items.push({ separator: true }, { label: 'Withdraw change request…', icon: 'x', disabled: busy !== '', action: () => onwithdraw?.('changes') });
    ctxMenu.show(e, items);
  }
</script>

<button class="btn small rv" class:primary class:rv-approved={!primary && review === 'approved'} class:rv-changes={!primary && review === 'changes'}
  aria-haspopup="menu" disabled={busy !== '' && busy !== 'comment'} data-testid="pr-review" data-review={review ?? 'none'} onclick={open}
  title={review === 'approved' ? 'Your review: approved' : review === 'changes' ? 'Your review: changes requested' : 'Approve, request changes or comment'}>
  {#if review === 'approved'}<Icon name="check" size={12} />{:else if review === 'changes'}<Icon name="warning" size={12} />{/if}
  {label} <Icon name="chevronDown" size={12} />
</button>

<style>
  .rv { gap: 4px; white-space: nowrap; }
  .rv-approved { color: var(--success); border-color: var(--success); background: var(--success-soft); }
  .rv-changes { color: var(--warning); border-color: var(--warning); background: var(--warning-soft); }
</style>
