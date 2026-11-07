<script lang="ts">
  // A pending approval, inline where the agent asked: what it wants to run /
  // change, its reason, then "Waiting for approval · Decline · Approve · More".
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/TimelinePendingApprovalRow.tsx and
  // ComposerPendingApprovalActions.tsx (accept / decline as buttons, every
  // other offered decision under "More"). `inline`: folded into the tool's
  // own row (TimelineWorkEntryRow), which already says what it runs.
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import type { ApprovalDecision, ApprovalOption, RequestKind } from '../../../lib/api/types';
  import type { PendingApproval } from './timeline';

  interface Props {
    approval: PendingApproval;
    onrespond: (requestId: string, decision: ApprovalDecision) => Promise<void>;
    inline?: boolean;
  }
  let { approval, onrespond, inline = false }: Props = $props();

  const LABELS: Record<RequestKind, string> = {
    command: 'Command approval',
    file_change: 'File change approval',
    file_read: 'File read approval',
    permission: 'Permission approval',
    question: 'Question',
    mcp_elicitation: 'App access approval',
  };
  const DEFAULT_OPTIONS: ApprovalOption[] = [
    { decision: 'decline', label: 'Decline' },
    { decision: 'accept', label: 'Approve' },
  ];

  let responding = $state(false);
  const options = $derived(approval.options.length > 0 ? approval.options : DEFAULT_OPTIONS);
  const primary = $derived(options.filter((o) => o.decision === 'decline' || o.decision === 'accept'));
  const more = $derived(options.filter((o) => o.decision !== 'decline' && o.decision !== 'accept'));

  async function respond(decision: ApprovalDecision): Promise<void> {
    if (responding || !approval.canRespond) return;
    responding = true;
    try {
      await onrespond(approval.requestId, decision);
    } finally {
      responding = false;
    }
  }

  function openMore(e: MouseEvent): void {
    ctxMenu.show(
      e,
      more.map((o) => ({ label: o.label, action: () => void respond(o.decision) })),
    );
  }
</script>

<div class="ra" class:inline role="group" aria-label={LABELS[approval.kind]} data-testid="runtime-approval-{approval.requestId}">
  {#if !inline}
    <div class="ra-head">
      <Icon name="shield" size={13} />
      <code class="ra-detail" dir="auto">{approval.prompt || LABELS[approval.kind]}</code>
    </div>
  {/if}
  {#if approval.reason}<p class="ra-reason" dir="auto">{approval.reason}</p>{/if}
  <div class="ra-actions">
    {#if approval.canRespond}
      <span class="ra-wait">Waiting for approval</span>
      {#each primary as o (o.decision)}
        <button
          type="button"
          class="btn small ra-btn"
          class:primary={o.decision === 'accept'}
          disabled={responding}
          onclick={() => void respond(o.decision)}
        >
          {o.label}
        </button>
      {/each}
      {#if more.length > 0}
        <button
          type="button"
          class="btn small ra-btn"
          aria-haspopup="menu"
          aria-label="More approval options"
          title="More approval options"
          disabled={responding}
          onclick={openMore}
        >
          More
        </button>
      {/if}
    {:else}
      <span class="ra-wait">The agent process is gone. Stop the turn to continue.</span>
    {/if}
  </div>
</div>

<style>
  .ra {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-block: 4px;
    padding: 8px 10px;
    border: 1px solid color-mix(in srgb, var(--warning) 40%, var(--border));
    border-radius: var(--radius-m);
    background: var(--warning-soft);
    font-size: var(--fs-s);
  }
  .ra.inline {
    margin-block: 2px 6px;
    margin-inline-start: 22px;
    padding: 6px 8px;
  }
  .ra.inline :is(.ra-reason, .ra-actions) {
    padding-inline-start: 0;
  }
  .ra-head {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    min-width: 0;
    color: var(--warning);
  }
  .ra-detail {
    flex: 1;
    min-width: 0;
    max-block-size: 160px;
    overflow: auto;
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  /* Phone: approving is the thread's main action — full 44 px tap targets. */
  @media (pointer: coarse) {
    .ra-btn {
      min-block-size: calc(var(--hit-min) + 8px);
      padding-inline: 14px;
    }
  }
  .ra-reason {
    margin: 0;
    padding-inline-start: 20px;
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .ra-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding-inline-start: 20px;
  }
  .ra-wait {
    margin-inline-end: 4px;
    color: var(--warning);
    font-size: var(--fs-xs);
  }
</style>
