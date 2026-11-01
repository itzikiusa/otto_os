<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import {
    formatEffortLabel,
    formatInteractionModeLabel,
    formatModelLabel,
    type EffortLevel,
    type InteractionMode,
  } from '../../../lib/providerModels';
  import { auth } from '../../../lib/stores/auth.svelte';

  interface Props {
    provider: string;
    effectiveModel: string | null;
    effort: EffortLevel | null;
    interactionMode: InteractionMode;
    statsSummary?: string;
    branch?: string | null;
    cwd?: string;
    onclose: () => void;
    onopenmodel: () => void;
    onopeneffort: () => void;
    onopenmode: () => void;
  }

  let {
    provider,
    effectiveModel,
    effort,
    interactionMode,
    statsSummary = '',
    branch = null,
    cwd = '',
    onclose,
    onopenmodel,
    onopeneffort,
    onopenmode,
  }: Props = $props();
</script>

<div class="status-card" data-testid="thread-status-card" role="region" aria-label="Thread status">
  <div class="sc-head">
    <span class="sc-title">Thread status</span>
    {#if statsSummary}
      <span class="sc-stats mono" dir="ltr">{statsSummary}</span>
    {/if}
    <button
      type="button"
      class="icon-btn"
      aria-label="Close status card"
      title="Close"
      onclick={onclose}
    >
      <Icon name="x" size={12} />
    </button>
  </div>
  <div class="sc-grid">
    <span class="sc-kv">
      <span class="dim">Model:</span>
      <button type="button" class="sc-link" onclick={onopenmodel}>
        {formatModelLabel(provider, effectiveModel, auth.meta)}
      </button>
    </span>
    <span class="sc-kv">
      <span class="dim">Effort:</span>
      <button type="button" class="sc-link" onclick={onopeneffort}>
        {formatEffortLabel(effort)}
      </button>
    </span>
    <span class="sc-kv">
      <span class="dim">Mode:</span>
      <button type="button" class="sc-link" onclick={onopenmode}>
        {formatInteractionModeLabel(interactionMode)}
      </button>
    </span>
    {#if branch}
      <span class="sc-kv mono" dir="ltr"><span class="dim">Branch:</span> {branch}</span>
    {/if}
    {#if cwd}
      <span class="sc-kv mono ellipsis" dir="ltr" title={cwd}><span class="dim">Folder:</span> {cwd}</span>
    {/if}
  </div>
</div>

<style>
  .status-card {
    max-width: var(--chat-measure, none);
    margin-inline: auto;
    width: 100%;
    margin-block-end: 8px;
    padding: 8px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface-2);
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-xs);
  }
  .sc-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .sc-title {
    font-weight: 600;
    color: var(--text);
  }
  .sc-stats {
    color: var(--text-dim);
    margin-inline-end: auto;
  }
  .sc-grid {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px 14px;
  }
  .sc-kv {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    max-width: 280px;
  }
  .sc-link {
    border: 0;
    padding: 0;
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-weight: 500;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 2px;
  }
</style>
