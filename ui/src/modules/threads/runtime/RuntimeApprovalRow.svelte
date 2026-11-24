<script lang="ts">
  // A pending approval. Its live controls live ONCE, in the attention dock
  // above the composer (`docked`: what it wants to run / change, its reason,
  // the decision buttons, with keyboard hints on desktop). In the timeline it
  // is one quiet line, "Waiting for your approval · <command>", that brings
  // that request up in the dock and focuses it.
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/TimelinePendingApprovalRow.tsx and
  // ComposerPendingApprovalActions.tsx (accept / decline as buttons, every
  // other offered decision under "More"). `inline`: folded into the tool's
  // own row (TimelineWorkEntryRow), which already says what it runs.
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import type { ApprovalDecision, ApprovalOption, RequestKind } from '../../../lib/api/types';
  import { keyHint } from './attentionDock';
  import type { PendingApproval } from './timeline';

  interface Props {
    approval: PendingApproval;
    onrespond?: (requestId: string, decision: ApprovalDecision) => Promise<void>;
    inline?: boolean;
    /** In the attention dock: the live controls. Otherwise the timeline's marker. */
    docked?: boolean;
    /** Show the ⌘↵ / ⌘⌫ hints (desktop; the dock decides). */
    hints?: boolean;
    mac?: boolean;
  }
  let { approval, onrespond, inline = false, docked = false, hints = false, mac = true }: Props = $props();

  const LABELS: Record<RequestKind, string> = {
    command: 'Command approval',
    file_change: 'File change approval',
    file_read: 'File read approval',
    permission: 'Permission approval',
    question: 'Question',
    mcp_elicitation: 'App access approval',
  };
  const DEFAULT_OPTIONS: ApprovalOption[] = [
    { decision: 'decline', label: 'Deny' },
    { decision: 'accept', label: 'Allow once' },
  ];
  /** One vocabulary for every approval — runtime, subagent, MCP, ACP: the
   *  provider's own label stays as the button's tooltip (its exact scope). */
  const WORDS: Partial<Record<ApprovalDecision, string>> = {
    accept: 'Allow once',
    accept_always: 'Always allow',
    accept_for_session: 'Always allow',
    decline: 'Deny',
    decline_always: 'Always deny',
    cancel: 'Deny and stop',
  };
  const word = (o: ApprovalOption): string => WORDS[o.decision] ?? o.label;

  let responding = $state(false);
  const options = $derived(approval.options.length > 0 ? approval.options : DEFAULT_OPTIONS);
  /** Allow once · Always allow (when offered) · Deny — the same layout everywhere; the rest under More. */
  const always = $derived(options.find((o) => o.decision === 'accept_always') ?? options.find((o) => o.decision === 'accept_for_session'));
  const primary = $derived(
    [options.find((o) => o.decision === 'accept'), always, options.find((o) => o.decision === 'decline')].filter((o): o is ApprovalOption => !!o),
  );
  const more = $derived(options.filter((o) => !primary.includes(o)));

  /** Also the dock's keyboard answer (one request in flight at a time). */
  export async function respond(decision: ApprovalDecision): Promise<void> {
    if (responding || !approval.canRespond || !onrespond) return;
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
      more.map((o) => ({ label: word(o), title: o.label, action: () => void respond(o.decision) })),
    );
  }

  /** "Answer": bring this request up in the dock (RuntimeAttentionDock listens on its thread). */
  function pick(e: MouseEvent): void {
    e.currentTarget?.dispatchEvent(new CustomEvent('rt-attention-pick', { detail: approval.requestId, bubbles: true }));
  }
</script>

{#if docked}
  <div class="ra ra-docked" role="group" aria-label={LABELS[approval.kind]} data-testid="runtime-approval-dock-{approval.requestId}">
    {#if approval.folded}
      {#if approval.reason}<p class="ra-reason" dir="auto">{approval.reason}</p>{/if}
      {#if approval.prompt}<details class="ra-more"><summary>Details</summary><code class="ra-detail" dir="auto">{approval.prompt}</code></details>{/if}
    {:else}
      {#if approval.prompt}<code class="ra-detail" dir="auto" title={approval.prompt}>{approval.prompt}</code>{/if}
      {#if approval.reason}<p class="ra-reason" dir="auto">{approval.reason}</p>{/if}
    {/if}
    <div class="ra-actions">
      {#if approval.canRespond}
        {#each primary as o (o.decision)}
          <button
            type="button"
            class="btn small ra-btn"
            class:primary={o.decision === 'accept'}
            disabled={responding}
            data-testid="runtime-dock-{o.decision}"
            aria-label={word(o)}
            title={o.label !== word(o) ? o.label : undefined}
            aria-keyshortcuts={hints && (o.decision === 'accept' || o.decision === 'decline') ? (o.decision === 'accept' ? 'Meta+Enter' : 'Meta+Backspace') : undefined}
            onclick={() => void respond(o.decision)}
          >
            {word(o)}{#if hints && (o.decision === 'accept' || o.decision === 'decline')}<kbd class="ra-kbd">{keyHint(o.decision === 'accept' ? 'accept' : 'decline', mac)}</kbd>{/if}
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
{:else}
  <!-- The timeline's record: one quiet line; the dock above the composer is the one place to answer. -->
  <div class="ra-line" class:inline role="group" aria-label={LABELS[approval.kind]} data-testid="runtime-approval-{approval.requestId}">
    {#if approval.canRespond}
      <button type="button" class="ra-pick" title="Answer it above the composer" data-testid="runtime-approval-answer" onclick={pick}>
        <Icon name="shield" size={12} />
        <span class="ra-wait">Waiting for your approval</span>
        {#if !inline && approval.prompt}<span class="ra-what ellipsis" dir="auto">· {approval.prompt}</span>{/if}
      </button>
    {:else}
      <span class="ra-wait">The agent process is gone. Stop the turn to continue.</span>
    {/if}
  </div>
{/if}

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
  .ra-docked {
    flex: 0 1 auto;
    min-block-size: 0;
    margin: 0;
    padding: 0;
    border: 0;
    background: transparent;
  }
  .ra-docked :is(.ra-reason, .ra-actions) {
    padding-inline-start: 0;
  }
  .ra-docked .ra-reason {
    flex: 0 1 auto;
    min-block-size: 0;
    overflow-y: auto;
  }
  .ra-docked .ra-actions {
    flex: none;
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
  .ra-docked .ra-detail {
    flex: none;
    /* The full arguments / preview, scrolling inside a capped block. */
    max-block-size: calc(1.5em * 8);
  }
  /* Phone: approving is the thread's main action — full 44 px tap targets. */
  @media (pointer: coarse) {
    .ra-btn {
      min-block-size: calc(var(--hit-min) + 8px);
      padding-inline: 14px;
    }
  }
  .ra-more summary {
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .ra-more[open] .ra-detail {
    display: block;
    max-block-size: 160px;
    margin-block-start: var(--sp-1);
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
  .ra-docked .ra-actions {
    justify-content: flex-end;
  }
  .ra-wait {
    margin-inline-end: 4px;
    color: var(--warning);
    font-size: var(--fs-xs);
  }
  .ra-line {
    display: flex;
    min-inline-size: 0;
    margin-block: 2px;
  }
  .ra-line.inline {
    margin-inline-start: 22px;
  }
  .ra-pick {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    max-inline-size: 100%;
    padding: 2px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--warning);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .ra-pick:hover {
    background: var(--hover);
  }
  .ra-pick:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .ra-what {
    min-inline-size: 0;
    color: var(--text-dim);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  @media (pointer: coarse) {
    .ra-pick {
      min-block-size: 44px;
    }
  }
  .ra-kbd {
    margin-inline-start: 6px;
    font: inherit;
    font-size: var(--fs-xs);
    opacity: 0.7;
  }
  @media (hover: none) {
    .ra-kbd {
      display: none;
    }
  }
</style>
