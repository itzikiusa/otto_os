<script lang="ts">
  // An agent's provider · model · effort, as two chips that open the Threads
  // model panel (lib/components/ModelPickerPopover.svelte). Empty = Default:
  // the provider's own setting, exactly as in Threads. Used by agent steps,
  // review reviewers and the summarizer (ux.md §9.3).
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { formatEffortLabel, formatModelLabel, normalizeEffort, supportsEffortSelection, effortsForModel } from '../../../lib/providerModels';
  import { agentProviders } from '../../../lib/providers';
  import { isRuntimeProvider } from '../../../lib/providerRegistry';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { providerName } from '../editor/nodeSummary';
  import type { StepModelValue } from '../editor/stepRules';

  const Picker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));

  interface Props {
    value: StepModelValue;
    /** What an empty provider runs on. */
    defaultProvider: string;
    onchange: (next: StepModelValue) => void;
    /** A reviewer row: its provider is the row's, only model and effort change. */
    lockProvider?: boolean;
    /** Continuing an earlier step's thread with no pick of its own: "Same as …". */
    sameAs?: string | null;
    /** One small chip (reviewer rows) instead of the two. */
    compact?: boolean;
    label?: string;
  }
  let { value, defaultProvider, onchange, lockProvider = false, sameAs = null, compact = false, label = 'Agent' }: Props = $props();

  const provider = $derived(value.provider || defaultProvider);
  const takesEffort = $derived(supportsEffortSelection(provider, auth.meta));
  const providers = $derived(agentProviders().filter((p) => isRuntimeProvider(p, auth.meta)));
  const same = $derived(!!sameAs && !value.provider && !value.model && !value.effort);
  const modelText = $derived(value.model ? formatModelLabel(provider, value.model, auth.meta) : 'Default');
  const effortText = $derived(value.effort ? formatEffortLabel(value.effort) : 'Default');
  let open = $state(false);
  let anchor = $state<HTMLButtonElement | null>(null);
  let resetNote = $state('');

  /** A new provider keeps only the model and effort it offers. */
  function pickProvider(p: string): void {
    const kept = normalizeEffort(value.effort);
    const effort = kept && supportsEffortSelection(p, auth.meta) && effortsForModel(p, null, auth.meta).includes(kept) ? kept : '';
    resetNote = value.effort && !effort ? `Effort reset: ${providerName(p)} has no ${formatEffortLabel(value.effort)} setting` : '';
    onchange({ provider: p === defaultProvider && !value.provider ? '' : p, model: '', effort });
  }
</script>

<div class="sm" class:compact>
  {#if same}
    <button type="button" class="sm-chip" bind:this={anchor} data-testid="wf-step-model" aria-label="{label}: same as {sameAs}" onclick={() => (open = true)}>
      Same as {sameAs}<Icon name="chevronDown" size={12} />
    </button>
  {:else}
    <button type="button" class="sm-chip" bind:this={anchor} data-testid="wf-step-model" aria-label="{label}: {providerName(provider)}, model {modelText}{compact ? `, effort ${effortText}` : ''}"
      onpointerenter={() => Picker.prefetch()} onclick={() => (open = true)}>
      {#if !compact}<ProviderIcon {provider} size={14} /><span class="sm-p">{providerName(provider)}</span><span class="sm-dim">·</span>{/if}
      <span class:sm-dim={!value.model}>{modelText}</span>
      {#if compact}<span class="sm-dim">·</span><span class:sm-dim={!value.effort}>{effortText}</span>{/if}
      <Icon name="chevronDown" size={12} />
    </button>
    {#if takesEffort && !compact}
      <button type="button" class="sm-chip" data-testid="wf-step-effort" aria-label="Effort: {effortText}" onclick={() => (open = true)}>
        <span class="sm-dim">Effort</span> {effortText}<Icon name="chevronDown" size={12} />
      </button>
    {/if}
  {/if}
</div>
{#if resetNote}<p class="node-hint">{resetNote}</p>{/if}

{#if open && anchor && Picker.component}
  <Picker.component
    {anchor}
    providers={lockProvider ? [provider] : providers}
    {provider}
    {lockProvider}
    model={value.model || null}
    effort={normalizeEffort(value.effort)}
    mode="full"
    onprovider={lockProvider ? undefined : pickProvider}
    onmodel={(m) => onchange({ ...value, model: m ?? '' })}
    oneffort={takesEffort ? (e) => onchange({ ...value, effort: e ?? '' }) : undefined}
    onclose={() => (open = false)}
  />
{/if}

<style>
  .sm {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
  }
  .sm-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-block-size: 30px;
    max-inline-size: 100%;
    padding: 4px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .sm-chip:hover {
    border-color: var(--border-strong);
  }
  .compact .sm-chip {
    min-block-size: 24px;
    padding: 2px 8px;
    font-size: var(--fs-xs);
  }
  .sm-p {
    font-weight: 500;
  }
  .sm-dim {
    color: var(--text-dim);
  }
</style>
