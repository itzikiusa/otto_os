<script lang="ts">
  // The end of the composer's tool row: (Threads) the quiet model · effort
  // chip, then Send — or, while the agent works, Stop / Steer / Queue.
  import Icon from '../../../lib/components/Icon.svelte';
  import ThreadModelBar from '../../../lib/components/ThreadModelBar.svelte';
  import type { EffortLevel, InteractionMode, SessionThreadConfig } from '../../../lib/providerModels';

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

  let bar = $state<ReturnType<typeof ThreadModelBar> | null>(null);
  export function openModelPicker(): void {
    bar?.openModelPicker();
  }
  export function openEffortPicker(): void {
    bar?.openEffortPicker();
  }
</script>

{#if chip && !busy}
  <ThreadModelBar
    bind:this={bar}
    {provider}
    parts={['model', 'effort']}
    quiet
    pinnedModel={cfg.pinnedModel}
    effectiveModel={cfg.effectiveModel}
    effort={cfg.effort}
    interactionMode={cfg.interactionMode}
    disabled={configBusy}
    {onmodelchange}
    {oneffortchange}
    {onmodechange}
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
