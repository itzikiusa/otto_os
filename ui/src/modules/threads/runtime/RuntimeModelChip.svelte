<script lang="ts">
  // The composer's model chip ("Claude Code · High"): opens the model and
  // effort picker for the next turn (RuntimeThread owns the popover).
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';

  interface Props {
    provider: string;
    label: string;
    expanded: boolean;
    disabled: boolean;
    showIcon: boolean;
    testId: string;
    onprefetch: () => void;
    onopen: () => void;
    el?: HTMLButtonElement | null;
  }
  let { provider, label, expanded, disabled, showIcon, testId, onprefetch, onopen, el = $bindable(null) }: Props = $props();
</script>

<button
  bind:this={el}
  type="button"
  class="rt-agent"
  aria-haspopup="dialog"
  aria-expanded={expanded}
  aria-label="Model for the next turn: {label}"
  title="Model and effort for the next turn"
  data-testid={testId}
  {disabled}
  onpointerenter={onprefetch}
  onclick={onopen}
>
  {#if showIcon && hasProviderIcon(provider)}<ProviderIcon {provider} size={13} />{/if}
  <span class="ellipsis">{label}</span>
</button>

<style>
  .rt-agent {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 100%;
    min-inline-size: 0;
    block-size: 28px;
    padding-inline: 6px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    font: inherit;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rt-agent:hover:not(:disabled) {
    background: var(--hover);
    color: var(--text);
  }
  .rt-agent:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  /* Touch: a taller target without growing the desktop chip. */
  @media (pointer: coarse) {
    .rt-agent {
      block-size: var(--hit-min);
    }
  }
</style>
