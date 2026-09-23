<script lang="ts">
  // Review status as a soft-tinted pill with a dot (colour + word, never colour
  // alone). With `onclick` it is the artifact header's status menu button.
  import Icon from '../../lib/components/Icon.svelte';
  import { statusLabel, statusTone } from './model';
  import type { DesignStatus } from '../../lib/api/types';

  interface Props {
    status: DesignStatus | string;
    onclick?: (e: MouseEvent) => void;
    /** Extra words after the label ("· v8"). */
    suffix?: string;
  }
  let { status, onclick, suffix = '' }: Props = $props();
  const tone = $derived(statusTone(status));
</script>

{#if onclick}
  <button
    class="pill tone-{tone} as-btn"
    {onclick}
    aria-haspopup="menu"
    title="Change status"
    data-testid="design-status"
  >
    <span class="dot" aria-hidden="true"></span>{statusLabel(status)}{suffix}
    <Icon name="chevronDown" size={12} />
  </button>
{:else}
  <span class="pill tone-{tone}"><span class="dot" aria-hidden="true"></span>{statusLabel(status)}{suffix}</span>
{/if}

<style>
  .pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    font-size: var(--fs-xs);
    font-weight: 500;
    white-space: nowrap;
    border: 1px solid transparent;
    color: var(--text-dim);
    background: var(--surface-2);
    border-color: var(--border);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    flex: none;
  }
  .tone-warn {
    color: var(--warning);
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 30%, transparent);
  }
  .tone-ok {
    color: var(--success);
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 30%, transparent);
  }
  .tone-info {
    color: var(--info);
    background: var(--info-soft);
    border-color: color-mix(in srgb, var(--info) 30%, transparent);
  }
  .tone-bad {
    color: var(--danger);
    background: var(--danger-soft);
    border-color: color-mix(in srgb, var(--danger) 30%, transparent);
  }
  .as-btn {
    cursor: pointer;
    font-family: inherit;
    transition: filter 130ms ease-out;
  }
  .as-btn:hover {
    filter: brightness(0.97);
  }
</style>
