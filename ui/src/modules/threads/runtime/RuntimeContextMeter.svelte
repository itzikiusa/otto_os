<script lang="ts">
  // How full the agent's context is, beside the model chip: a calm ring
  // (--warning at 80 %, --danger at 95 %). Its menu says the numbers and,
  // when the agent publishes a compact command, offers it (contextMeter.ts).
  import type { ThreadTokenUsage } from '../../../lib/api/types';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { contextMeter } from './contextMeter';

  interface Props {
    usage: ThreadTokenUsage | null | undefined;
    /** The agent's own compact command, when it has one (`compact`). */
    compact?: string | null;
    oncompact?: (command: string) => void;
  }
  let { usage, compact = null, oncompact }: Props = $props();
  const m = $derived(contextMeter(usage));
  const R = 7;
  const C = 2 * Math.PI * R;

  function open(e: MouseEvent): void {
    if (!m) return;
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: `Context · ${m.text}`, disabled: true },
      ...(compact && oncompact ? [{ label: 'Compact the conversation', icon: 'fold', hint: `/${compact}`, title: 'The agent summarizes the conversation so far and frees its context', action: () => oncompact(compact) }] : []),
    ]);
  }
</script>

{#if m}
  <button
    type="button"
    class="cm {m.tone}"
    aria-haspopup="menu"
    aria-label="Context {m.pct} % full: {m.text}"
    title="Context: {m.text}"
    data-testid="runtime-context-meter"
    data-tone={m.tone}
    onclick={open}
  >
    <svg viewBox="0 0 18 18" width="16" height="16" aria-hidden="true">
      <circle cx="9" cy="9" r={R} class="cm-track" />
      <circle cx="9" cy="9" r={R} class="cm-fill" stroke-dasharray="{(m.pct / 100) * C} {C}" transform="rotate(-90 9 9)" />
    </svg>
    {#if m.tone !== 'ok'}<span class="cm-pct">{m.pct}%</span>{/if}
  </button>
{/if}

<style>
  .cm {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    block-size: 28px;
    padding-inline: var(--sp-1);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .cm:hover {
    background: var(--hover);
  }
  .cm:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .cm-track,
  .cm-fill {
    fill: none;
    stroke-width: 2.5;
  }
  .cm-track {
    stroke: var(--border);
  }
  .cm-fill {
    stroke: var(--text-dim);
    stroke-linecap: round;
  }
  .cm.warning,
  .cm.warning .cm-fill {
    color: var(--warning);
    stroke: var(--warning);
  }
  .cm.danger,
  .cm.danger .cm-fill {
    color: var(--danger);
    stroke: var(--danger);
  }
  @media (pointer: coarse) {
    .cm {
      block-size: calc(var(--hit-min) + 8px);
      min-inline-size: calc(var(--hit-min) + 8px);
      justify-content: center;
    }
  }
</style>
