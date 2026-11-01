<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';

  interface Props {
    variant: 'ready' | 'plan-bar';
    effectiveProvider?: string;
    agentName?: string;
    shortThreadCwd?: string;
    canCompose?: boolean;
    implementingPlan?: boolean;
    canSpawnThread?: boolean;
    onreuseprompt?: (prompt: string) => void;
    onimplementhere?: () => void;
    onimplementbeside?: () => void;
  }

  let {
    variant,
    effectiveProvider = 'claude',
    agentName = 'Agent',
    shortThreadCwd = 'workspace',
    canCompose = true,
    implementingPlan = false,
    canSpawnThread = false,
    onreuseprompt,
    onimplementhere,
    onimplementbeside,
  }: Props = $props();
</script>

{#if variant === 'ready'}
  <div class="thread-ready" data-testid="thread-ready-state">
    <div class="tr-icon" aria-hidden="true">
      {#if effectiveProvider && hasProviderIcon(effectiveProvider)}
        <ProviderIcon provider={effectiveProvider} size={24} />
      {:else}
        <Icon name="sparkle" size={24} />
      {/if}
    </div>
    <h3 class="tr-title">{agentName} ready in <span class="mono" dir="ltr">{shortThreadCwd}</span></h3>
    <p class="tr-sub">
      Pick a model, effort, or Plan mode below — or type <span class="mono">/</span> for interactive commands.
    </p>
    {#if canCompose && onreuseprompt}
      <div class="tr-prompts">
        <button
          type="button"
          class="btn small ghost"
          onclick={() => onreuseprompt('Inspect uncommitted changes and highlight any bugs or risks.')}
        >
          <Icon name="sparkle" size={12} /> Review changes
        </button>
        <button
          type="button"
          class="btn small ghost"
          onclick={() => onreuseprompt('Explore this codebase and propose a clear step-by-step implementation plan.')}
        >
          <Icon name="layers" size={12} /> Plan a feature
        </button>
        <button
          type="button"
          class="btn small ghost"
          onclick={() => onreuseprompt('Run the relevant test suite and diagnose any failures.')}
        >
          <Icon name="check" size={12} /> Run &amp; fix tests
        </button>
      </div>
    {/if}
  </div>
{:else}
  <div class="plan-bar" data-testid="plan-action-bar" role="region" aria-label="Plan actions">
    <div class="plan-bar-copy">
      <Icon name="layers" size={14} />
      <span>Plan ready — switch to Full access to implement it here, or hand off to a sibling thread.</span>
    </div>
    <div class="plan-bar-actions">
      {#if canSpawnThread && onimplementbeside}
        <button
          type="button"
          class="btn small"
          disabled={implementingPlan}
          data-testid="implement-plan-beside-btn"
          onclick={onimplementbeside}
        >
          <Icon name="split" size={12} />
          Implement in new thread
        </button>
      {/if}
      <button
        type="button"
        class="btn small primary"
        disabled={implementingPlan}
        data-testid="implement-plan-btn"
        onclick={onimplementhere}
      >
        <Icon name="check" size={12} />
        {implementingPlan ? 'Starting…' : 'Implement plan'}
      </button>
    </div>
  </div>
{/if}

<style>
  .thread-ready {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    gap: 10px;
    padding-block: 48px 32px;
    padding-inline: 20px;
    max-width: 520px;
    margin-inline: auto;
  }
  .tr-icon {
    inline-size: 44px;
    block-size: 44px;
    border-radius: var(--radius-l);
    background: var(--surface);
    border: 1px solid var(--border);
    display: grid;
    place-items: center;
    color: var(--accent-text);
  }
  .tr-title {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    color: var(--text);
  }
  .tr-sub {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .tr-prompts {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
    margin-block-start: 4px;
  }
  .plan-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
    padding-block: 8px;
    padding-inline: clamp(12px, 3.2cqi, 40px);
    background: color-mix(in srgb, var(--warning) 8%, var(--surface));
    border-block-start: 1px solid color-mix(in srgb, var(--warning) 35%, var(--border));
    flex-shrink: 0;
  }
  .plan-bar-copy {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text);
    min-width: 0;
  }
  .plan-bar-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-inline-start: auto;
  }
</style>
