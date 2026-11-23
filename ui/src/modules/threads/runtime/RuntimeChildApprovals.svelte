<!--
  A subagent's approval, answered in its parent (T3 Code's
  ChildThreadApprovals.tsx, MIT, Copyright (c) 2026 T3 Tools Inc.):
  "Approval needed from <title>" above the composer with the same Approve /
  Decline as the thread's own, sent through the subagent's own request route.
  Only subagents whose row says they wait on the person are loaded.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import type { ApprovalDecision } from '../../../lib/api/types';
  import { toastError } from '../../../lib/toastError';
  import { childrenOf } from './childThreads';
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { pendingApprovals } from './timeline';

  let { sessionId }: { sessionId: string } = $props();

  const waiting = $derived(childrenOf(Object.values(runtimeThreads.byId), sessionId).filter((k) => k.status === 'awaiting_permission'));
  const waitingIds = $derived(waiting.map((k) => k.id).join(','));
  // Load (and follow) only the subagents that wait on the person.
  $effect(() => {
    const ids = waitingIds ? waitingIds.split(',') : [];
    const closers = untrack(() => ids.map((id) => runtimeViews.open(id)));
    return () => closers.forEach((c) => c?.());
  });
  const items = $derived(
    waiting.flatMap((k) => {
      const p = runtimeViews.projections[k.id];
      return p ? pendingApprovals(p).filter((a) => a.kind !== 'question').map((approval) => ({ kid: k, approval })) : [];
    }),
  );
  let at = $state(0);
  const cur = $derived(items.length ? items[Math.min(at, items.length - 1)] : null);

  async function answer(requestId: string, decision: ApprovalDecision): Promise<void> {
    if (!cur) return;
    try {
      await runtimeThreads.respond(cur.kid.id, requestId, decision);
    } catch (e) {
      toastError('Couldn’t answer the approval', e);
    }
  }
</script>

{#if cur}
  <div class="ca" role="region" aria-label="Approval needed from {cur.kid.title}" data-testid="runtime-child-approval" data-child={cur.kid.id}>
    <div class="ca-head">
      {#if hasProviderIcon(cur.kid.provider)}<ProviderIcon provider={cur.kid.provider} size={13} />{:else}<Icon name="shield" size={13} />{/if}
      <span class="ca-title ellipsis" dir="auto"><strong>Approval needed from {cur.kid.title}</strong></span>
      {#if items.length > 1}
        <button type="button" class="btn small ghost" data-testid="runtime-child-approval-next" onclick={() => (at = (at + 1) % items.length)}>{Math.min(at, items.length - 1) + 1} of {items.length}</button>
      {/if}
    </div>
    {#key cur.approval.requestId}
      <RuntimeApprovalRow approval={cur.approval} onrespond={answer} docked />
    {/key}
  </div>
{/if}

<style>
  .ca {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-inline: 12px;
    padding: 8px 12px;
    border: 1px solid var(--warning);
    border-radius: var(--radius-l);
    background: var(--surface);
  }
  .ca-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    font-size: var(--fs-m);
  }
  .ca-title {
    flex: 1;
    min-inline-size: 0;
  }
</style>
