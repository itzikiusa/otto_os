<!--
  "Ask several" in the new-thread stage (a council: one prompt to 2–4
  agents). `part="toggle"` is the switch beside the agent chip; with it on,
  every model picked in the agent menu (or ⌘ / Shift-clicked with it off)
  becomes a seat. `part="seats"` lists the picked seats above the composer
  foot, each removable.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import type { EffortLevel } from '../../../lib/providerModels';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { COUNCIL_MAX_SEATS, COUNCIL_MIN_SEATS } from './council';
  import { council, seatLabel } from './councilStore.svelte';
  import { isRuntimeProvider } from './runtimeThreads.svelte';

  interface Props {
    part: 'toggle' | 'seats';
    provider: string;
    model: string | null;
    effort: EffortLevel | null;
    disabled?: boolean;
  }
  let { part, provider, model, effort, disabled = false }: Props = $props();

  const runtime = $derived(isRuntimeProvider(provider));
</script>

{#if part === 'toggle'}
  <button
    type="button"
    class="cp-toggle"
    class:on={council.on}
    aria-pressed={council.on}
    aria-label="Ask several agents"
    disabled={disabled || (!runtime && !council.on)}
    title={!runtime && !council.on
      ? 'Asking several agents works with Claude, Codex, Gemini, Grok, opencode and pi'
      : 'Ask several: send this prompt to 2–4 agents and compare (⌘ or ⇧-click models to add them)'}
    data-testid="new-thread-ask-several"
    onclick={() => council.toggle({ provider, model, effort })}
  >
    <Icon name="people" size={14} />
    <span class="cp-toggle-label">Ask several</span>
  </button>
{:else if council.on}
  <div class="cp-seats" role="group" aria-label="Agents to ask" data-testid="new-thread-council-seats">
    {#each council.seats as seat, i (`${seat.provider}:${seat.model}:${seat.effort}`)}
      <span class="cp-seat">
        {#if hasProviderIcon(seat.provider)}<ProviderIcon provider={seat.provider} size={12} />{/if}
        <span class="ellipsis">{seatLabel(seat, auth.meta)}</span>
        <button
          type="button"
          class="icon-btn cp-remove"
          aria-label="Remove {seatLabel(seat, auth.meta)}"
          title="Remove this agent"
          onclick={() => council.remove(i)}
        >
          <Icon name="x" size={12} />
        </button>
      </span>
    {/each}
    <span class="cp-hint">
      {council.seats.length < COUNCIL_MIN_SEATS
        ? 'Pick another model in the agent menu — the same provider can sit twice.'
        : council.full
          ? `${COUNCIL_MAX_SEATS} agents is the most.`
          : 'Pick more models in the agent menu, or send.'}
    </span>
  </div>
{/if}

<style>
  .cp-toggle {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: none;
    block-size: 30px;
    padding-inline: 8px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .cp-toggle:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .cp-toggle.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .cp-toggle:disabled {
    cursor: default;
    opacity: var(--disabled-opacity);
  }
  .cp-seats {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
  }
  .cp-seat {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-inline-size: 100%;
    min-inline-size: 0;
    block-size: 26px;
    padding-inline: 8px 2px;
    border: 1px solid var(--accent-line);
    border-radius: 999px;
    background: var(--accent-faint);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .cp-remove {
    inline-size: 22px;
    block-size: 22px;
  }
  .cp-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .cp-toggle-label {
      display: none;
    }
  }
</style>
