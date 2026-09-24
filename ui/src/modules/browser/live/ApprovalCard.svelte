<script lang="ts">
  // An agent's outward action waiting for a person, drawn over the live
  // frame (plan §4.6, mockup panel 1). The one approval shape
  // (docs/design/guidelines/patterns.md §5): WHO asked, WHAT (where / what /
  // who sees it), WHY, WHEN — then Approve once (primary) · Always for this
  // site · Take over · Deny. Nothing here decides on its own: every button is
  // an explicit decision sent back to the daemon, which records it.
  //
  // Not a Modal: it blocks only the agent (which is parked on it), not the
  // rest of Otto, so it sits in the live pane and the user can still scroll
  // the step log, switch tabs, or read the page behind it.
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import type { ApprovalDecision, LiveApproval } from './protocol';

  interface Props {
    approval: LiveApproval;
    ondecide: (decision: ApprovalDecision) => void;
    /** A decision is in flight (buttons disabled, labels say so). */
    busy?: boolean;
  }
  let { approval, ondecide, busy = false }: Props = $props();

  let cardEl = $state<HTMLElement | null>(null);

  // Move focus to the card (not onto Approve — an Enter meant for the page
  // must never approve by accident) so screen readers announce it and the
  // keyboard lands next to the decisions.
  $effect(() => {
    const _id = approval.id;
    cardEl?.focus({ preventScroll: true });
  });

  const titleId = $derived(`approval-title-${approval.id}`);
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
      <h2 id={titleId}>{approval.title}</h2>
      <p class="summary">
        {approval.summary}
        {#if approval.reason}<span class="dim"> {approval.reason}</span>{/if}
      </p>
    </div>
  </div>

  <dl class="facts">
    <dt>Where</dt>
    <dd dir="auto">{approval.where}</dd>
    <dt>What</dt>
    <dd>{approval.what}</dd>
    <dt>Who sees it</dt>
    <dd>{approval.who_sees}</dd>
    {#if approval.as_profile}
      <dt>As</dt>
      <dd>{approval.as_profile}</dd>
    {/if}
  </dl>

  {#if approval.screenshot_url}
    <figure class="shot">
      <img src={approval.screenshot_url} alt="The page just before the action, with the target outlined" />
      <figcaption>Screenshot taken just before the action, with the target outlined. Saved to the run trace.</figcaption>
    </figure>
  {/if}

  <p class="who">
    {#if approval.provider}<ProviderIcon provider={approval.provider} size={12} />{/if}
    <span>{approval.agent_name || 'An agent'} asked</span>
    <span aria-hidden="true">·</span>
    <time datetime={approval.requested_at} title={new Date(approval.requested_at).toLocaleString()}>{rel(approval.requested_at)}</time>
  </p>

  <div class="actions">
    <button class="btn primary" disabled={busy} onclick={() => ondecide('approve_once')}>Approve once</button>
    <button class="btn" disabled={busy} onclick={() => ondecide('always_site')}>Always for this site</button>
    <button class="btn" disabled={busy} onclick={() => ondecide('take_over')}>Take over</button>
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
  }
  .summary {
    margin: 0;
    color: var(--text);
    line-height: 1.45;
  }
  .dim {
    color: var(--text-dim);
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
  .shot {
    display: flex;
    gap: 12px;
    align-items: center;
    margin: 14px 0 0;
    padding: 8px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .shot img {
    flex: none;
    width: 92px;
    height: 56px;
    object-fit: cover;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--bg);
  }
  figcaption {
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
