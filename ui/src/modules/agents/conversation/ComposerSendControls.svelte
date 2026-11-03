<script lang="ts">
  // The end of the composer's tool row: (Threads) the quiet model · effort
  // chip, then Send — or, while the agent works, Stop / Steer / Queue.
  import Icon from '../../../lib/components/Icon.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  // The model panel loads on first open (prefetched when the chip is hovered).
  const ModelPicker = lazyComponent(() => import('../../../lib/components/ModelPickerPopover.svelte'));
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import {
    formatEffortShort,
    formatModelLabel,
    supportsModelSelection,
    type EffortLevel,
    type InteractionMode,
    type SessionThreadConfig,
  } from '../../../lib/providerModels';
  import { agentProviders, harnessLabel } from '../../../lib/providers';
  import { auth } from '../../../lib/stores/auth.svelte';

  interface Props {
    /** Show the model · effort chip in the box (Threads). */
    chip: boolean;
    provider: string;
    cfg: SessionThreadConfig;
    configBusy: boolean;
    busy: boolean;
    canSend: boolean;
    stopping: boolean;
    sendTitle: string;
    uploading: number;
    agentName: string;
    onmodelchange: (m: string | null) => void;
    oneffortchange: (e: EffortLevel | null) => void;
    onmodechange: (m: InteractionMode) => void;
    onstop: () => void;
    onsteer: () => void;
    onsend: () => void;
  }
  let {
    chip,
    provider,
    cfg,
    configBusy,
    busy,
    canSend,
    stopping,
    sendTitle,
    uploading,
    agentName,
    onmodelchange,
    oneffortchange,
    onmodechange,
    onstop,
    onsteer,
    onsend,
  }: Props = $props();

  // Threads: one chip → one panel (models · reasoning · access), harness locked
  // to the session's own.
  let chipEl = $state<HTMLButtonElement | null>(null);
  let open = $state(false);
  export function openModelPicker(): void {
    if (chipEl) open = true;
  }
  export function openEffortPicker(): void {
    openModelPicker();
  }
  const chipLabel = $derived.by(() => {
    const m = cfg.effectiveModel ? formatModelLabel(provider, cfg.effectiveModel, auth.meta) : harnessLabel(provider);
    return cfg.effort ? `${m} · ${formatEffortShort(cfg.effort)}` : m;
  });
</script>

{#if chip && !busy && supportsModelSelection(provider, auth.meta)}
  <button
    bind:this={chipEl}
    type="button"
    class="csc-chip"
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label="Model and reasoning: {chipLabel}"
    title="Model, reasoning and access (/model · /effort)"
    data-testid="picker-model"
    onpointerenter={() => ModelPicker.prefetch()}
    disabled={configBusy}
    onclick={openModelPicker}
  >
    <ProviderIcon {provider} size={13} />
    <span class="ellipsis">{chipLabel}</span>
  </button>
{/if}
{#if open && chipEl && ModelPicker.component}
  <ModelPicker.component
    anchor={chipEl}
    providers={agentProviders().includes(provider) ? agentProviders() : [provider, ...agentProviders()]}
    {provider}
    lockProvider
    model={cfg.pinnedModel}
    effectiveModel={cfg.effectiveModel}
    effort={cfg.effort}
    mode={cfg.interactionMode}
    onmodel={onmodelchange}
    oneffort={oneffortchange}
    onmode={onmodechange}
    onclose={() => (open = false)}
  />
{/if}
{#if uploading > 0}<span class="uploading">Uploading…</span>{/if}
{#if busy}
  <button class="btn small stop" onclick={onstop} disabled={stopping} title="Interrupt {agentName} — sends Esc to its terminal"><Icon name="stop" size={12} /> Stop</button>
  <button
    class="btn small steer-btn"
    onclick={onsteer}
    disabled={!canSend}
    title="Steer {agentName} (⇧⌘⏎) — interrupt the current step and send this prompt immediately"
  >
    <Icon name="zap" size={12} /> Steer
  </button>
  <button class="btn small primary queue-btn" onclick={onsend} disabled={!canSend} title={sendTitle}>
    <Icon name="send" size={12} /> Queue
  </button>
{:else}
  <button class="send" onclick={onsend} disabled={!canSend} title={sendTitle} aria-label={uploading > 0 ? 'Send (uploading images)' : 'Send'}><Icon name={chip ? 'arrowUp' : 'send'} size={14} /></button>
{/if}

<style>
  .csc-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 240px;
    block-size: 28px;
    padding-inline: 8px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .csc-chip:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .csc-chip:disabled {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  .stop,
  .steer-btn,
  .queue-btn {
    flex-shrink: 0;
  }
  .steer-btn {
    border-color: var(--accent-line);
    color: var(--accent-text);
    background: var(--accent-faint);
  }
  .steer-btn:hover:not(:disabled) {
    background: var(--accent-soft);
  }
  .uploading {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .send {
    flex-shrink: 0;
    display: inline-grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 50%;
    border: 0;
    padding: 0;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    cursor: pointer;
  }
  .send:hover:not(:disabled) {
    filter: brightness(1.08);
  }
  .send:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .send:disabled {
    background: var(--surface-3);
    color: var(--text-dim);
    cursor: default;
  }
  @media (max-width: 640px), (pointer: coarse) {
    .send {
      width: var(--hit-min);
      height: var(--hit-min);
    }
  }
</style>
