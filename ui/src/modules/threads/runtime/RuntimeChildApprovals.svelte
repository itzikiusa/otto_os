<!--
  A subagent's approval, answered in its parent (T3 Code's
  ChildThreadApprovals.tsx, MIT, Copyright (c) 2026 T3 Tools Inc.):
  "Approval needed from <title>" above the composer with the same Approve /
  Decline as the thread's own, sent through the subagent's own request route.
  A subagent's QUESTION goes to this thread's agent first
  (runtime/child_question.rs); once that agent's turn ended without
  answering it, it shows here as "Question from <title>". Only subagents
  whose row says they wait on someone are loaded.
-->
<script lang="ts">
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import type { ApprovalDecision } from '../../../lib/api/types';
  import { toastError } from '../../../lib/toastError';
  import { childrenOf } from './childThreads';
  import { mcpAsks } from './mcpAsks.svelte';
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { pendingApprovals } from './timeline';
  import { QuestionCardView } from '../threadLazy.svelte';

  let { sessionId }: { sessionId: string } = $props();

  // Only this family's waiting subagents, as a key: any other row's update
  // leaves it equal, so nothing below re-runs (performance).
  const waitingIds = $derived(
    childrenOf(Object.values(runtimeThreads.byId), sessionId)
      // A held MCP call does not move the subagent to awaiting_permission.
      .filter((k) => k.status === 'awaiting_permission' || mcpAsks.waiting(k.id))
      .map((k) => k.id)
      .join(','),
  );
  const waiting = $derived(waitingIds ? waitingIds.split(',').flatMap((id) => runtimeThreads.get(id) ?? []) : []);
  // Load (and follow) only the subagents that wait on the person.
  $effect(() => {
    const ids = waitingIds ? waitingIds.split(',') : [];
    const closers = untrack(() => ids.map((id) => runtimeViews.open(id)));
    return () => closers.forEach((c) => c?.());
  });
  /** This thread's agent is done with its turn: a question it did not answer is the person's now. */
  const escalated = $derived(runtimeThreads.get(sessionId)?.status !== 'working');
  const items = $derived(
    waiting.flatMap((k) => {
      const mcp = mcpAsks.rows(k.id).map((approval) => ({ kid: k, approval, question: null }));
      const p = runtimeViews.projections[k.id];
      if (!p) return mcp;
      const approvals = pendingApprovals(p)
        .filter((a) => a.kind !== 'question')
        .map((approval) => ({ kid: k, approval, question: null }));
      // An elicitation (`for_person`) never goes to this thread's agent: the person's at once.
      const questions = p.requests
        .filter((r) => r.status === 'pending' && r.kind === 'question' && r.response_capability.type === 'live' && (escalated || r.for_person))
        .map((question) => ({ kid: k, approval: null, question }));
      return [...mcp, ...approvals, ...questions];
    }),
  );
  let at = $state(0);
  const cur = $derived(items.length ? items[Math.min(at, items.length - 1)] : null);

  const curId = $derived(cur ? (cur.approval?.requestId ?? cur.question?.id ?? '') : '');

  async function answer(requestId: string, decision: ApprovalDecision): Promise<void> {
    if (!cur) return;
    try {
      const kid = cur.kid.id;
      await mcpAsks.respond(requestId, decision, () => runtimeThreads.respond(kid, requestId, decision));
    } catch (e) {
      toastError('Couldn’t answer the approval', e);
    }
  }
</script>

{#if cur}
  {@const what = cur.question ? 'Question from' : 'Approval needed from'}
  <div class="ca" role="region" aria-label="{what} {cur.kid.title}" data-testid={cur.question ? 'runtime-child-question' : 'runtime-child-approval'} data-child={cur.kid.id}>
    <div class="ca-head">
      {#if hasProviderIcon(cur.kid.provider)}<ProviderIcon provider={cur.kid.provider} size={13} />{:else}<Icon name="shield" size={13} />{/if}
      <span class="ca-title ellipsis" dir="auto"><strong>{what} {cur.kid.title}</strong></span>
      {#if items.length > 1}
        <button type="button" class="btn small ghost" data-testid="runtime-child-approval-next" onclick={() => (at = (at + 1) % items.length)}>{Math.min(at, items.length - 1) + 1} of {items.length}</button>
      {/if}
    </div>
    {#key curId}
      {#if cur.approval}
        <RuntimeApprovalRow approval={cur.approval} onrespond={answer} docked />
      {:else if cur.question}
        {#if QuestionCardView.component}<QuestionCardView.component request={cur.question} sessionId={cur.kid.id} agent={cur.kid.title} onanswered={() => {}} docked />{/if}
      {/if}
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
