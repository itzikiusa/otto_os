<script lang="ts">
  // An agent's outward action held for a person, drawn over the live frame
  // (plan §4.6, mockup panel 1). While an agent drives, the daemon holds any
  // state-changing document request (a form submit / non-GET navigation),
  // captures a screenshot and files an MCP approval (`kind:"browser_action"`);
  // the live socket announces it with an `approval` frame.
  //
  // The one approval shape (docs/design/guidelines/patterns.md §5): WHO asked,
  // WHAT (where / what / who sees it), WHY, WHEN — then Approve (primary) ·
  // Take over · Deny. Every button is an explicit decision recorded in the
  // MCP approvals queue; nothing is decided silently.
  //
  // Not a Modal: it parks only the agent, not the rest of Otto, so it sits in
  // the live pane and the user can still switch tabs or read the page.
  import Icon from '../../../lib/components/Icon.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import type { McpApproval } from '../../../lib/api/types';
  import { approvalFacts, type ApprovalChoice } from './approvalFacts';

  interface Props {
    /** The approval id + title from the `approval` frame. */
    id: string;
    title: string;
    /** Full queue row, when this viewer can read the queue (mcp:view). */
    detail: McpApproval | null;
    /** The live session's profile (the "As" line). */
    profile: string | null;
    ondecide: (choice: ApprovalChoice) => void;
    /** A decision is in flight (buttons disabled). */
    busy?: boolean;
    /** Why the last decision failed, inline. */
    error?: string;
  }
  let { id, title, detail, profile, ondecide, busy = false, error = '' }: Props = $props();

  let cardEl = $state<HTMLElement | null>(null);

  // Move focus to the card (not onto Approve — an Enter meant for the page
  // must never approve by accident) so screen readers announce it and the
  // keyboard lands next to the decisions.
  $effect(() => {
    const _id = id;
    cardEl?.focus({ preventScroll: true });
  });

  const facts = $derived(approvalFacts(detail, profile));
  const titleId = $derived(`approval-title-${id}`);
</script>

<div
  class="approval"
  role="alertdialog"
  aria-modal="false"
  aria-labelledby={titleId}
  tabindex="-1"
  bind:this={cardEl}
  data-testid="live-approval"
>
  <div class="head">
    <span class="tile" aria-hidden="true"><Icon name="warning" size={16} /></span>
    <div class="head-text">
      <h2 id={titleId}>{title}</h2>
      <p class="summary">
        The agent driving this page wants to send something from it. Otto held the request until you decide.
      </p>
    </div>
  </div>

  {#if facts.rows.length}
    <dl class="facts">
      {#each facts.rows as row (row.label)}
        <dt>{row.label}</dt>
        <dd dir={row.ltr ? 'ltr' : undefined} class:mono={row.ltr}>{row.value}</dd>
      {/each}
    </dl>
  {/if}

  {#if facts.why}
    <p class="why"><span class="dim">Agent's reason:</span> {facts.why}</p>
  {/if}

  {#if facts.screenshot}
    <p class="shot">
      <Icon name="image" size={14} />
      <span>A screenshot from just before the request is saved with this approval.</span>
    </p>
  {/if}

  <p class="who">
    <span class="chip">Agent</span>
    <span title={detail?.requested_by ?? undefined}>{facts.requester}</span>
    {#if detail?.created_at}
      <span aria-hidden="true">·</span>
      <time datetime={detail.created_at} title={new Date(detail.created_at).toLocaleString()}>{rel(detail.created_at)}</time>
    {/if}
  </p>

  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}

  <div class="actions">
    <button class="btn primary" disabled={busy} onclick={() => ondecide('approve')}>Approve</button>
    <button
      class="btn"
      disabled={busy}
      onclick={() => ondecide('take_over')}
      title="Deny the agent's request and drive the page yourself"
    >
      Take over
    </button>
    <span class="grow"></span>
    <button class="btn danger" disabled={busy} onclick={() => ondecide('deny')}>Deny…</button>
  </div>
</div>

<style>
  .approval {
    width: min(440px, 100%);
    max-height: 100%;
    overflow-y: auto;
    padding: 16px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    box-shadow: var(--shadow);
    color: var(--text);
    font-size: var(--fs-m);
  }
  .approval:focus {
    outline: none;
  }
  .approval:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .head {
    display: flex;
    gap: 12px;
    align-items: flex-start;
  }
  .tile {
    flex: none;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--warning);
  }
  .head-text {
    min-width: 0;
  }
  h2 {
    margin: 0 0 2px;
    font-size: var(--fs-l);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .summary {
    margin: 0;
    color: var(--text-dim);
    line-height: 1.45;
  }
  .facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 6px 16px;
    margin: 14px 0 0;
    font-size: var(--fs-s);
  }
  dt {
    color: var(--text-dim);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  dd.mono {
    font-family: var(--font-mono);
  }
  .why {
    margin: 12px 0 0;
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
  }
  .shot {
    display: flex;
    gap: 8px;
    align-items: center;
    margin: 12px 0 0;
    padding: 8px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 12px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .who .chip {
    color: var(--text-dim);
  }
  .error {
    margin: 10px 0 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .actions {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-top: 12px;
  }
  .grow {
    flex: 1;
  }
</style>
