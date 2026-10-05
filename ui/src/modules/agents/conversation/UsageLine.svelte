<script lang="ts">
  // A response's footer facts: how long it worked, on which model, and where
  // its tokens went — input (your side: blue), thinking (violet), output (the
  // agent's side: green), cache read / write (dim). Every figure carries a
  // tooltip that says what it counts. Renders nothing it does not know: no
  // usage recorded → no token pieces.
  import { fmtDuration, fmtTokens, usageParts } from './format';
  import type { TurnUsage } from '../../../lib/api/types';

  interface Props {
    usage: TurnUsage | null;
    durationMs?: number | null;
    model?: string | null;
  }
  let { usage, durationMs = null, model = null }: Props = $props();
  const parts = $derived(usageParts(usage));
  const dur = $derived(fmtDuration(durationMs));
  const total = $derived(usage ? usage.input_tokens + usage.output_tokens + usage.cache_read_tokens + usage.cache_creation_tokens : 0);
</script>

{#if parts.length || dur || model}
  <span class="usage" data-usage={usage ? total : undefined}>
    {#if dur}<span class="u-dur" title="How long the agent worked on this response">{dur}</span>{/if}
    {#if model}<span class="u-model mono" title="Model">{model}</span>{/if}
    {#each parts as p (p.key)}
      <span class="u-part u-{p.key}" title={`${p.title}: ${p.value.toLocaleString()}`} data-part={p.key}>
        <span class="u-dot" aria-hidden="true"></span>{fmtTokens(p.value) || '0'} <span class="u-lbl">{p.label}</span>
      </span>
    {/each}
  </span>
{/if}

<style>
  .usage {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px 10px;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .u-model {
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 22ch;
  }
  .u-part {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
    cursor: default;
  }
  .u-dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .u-part.u-input .u-dot {
    background: var(--accent);
  }
  .u-part.u-thinking {
    color: color-mix(in srgb, var(--cat-4) 70%, var(--text));
    font-style: italic;
  }
  .u-part.u-thinking .u-dot {
    background: var(--cat-4);
  }
  .u-part.u-output .u-dot {
    background: var(--status-working);
  }
  .u-part.u-cache_read .u-dot,
  .u-part.u-cache_write .u-dot {
    background: transparent;
    border: 1px solid var(--text-dim);
  }
  .u-lbl {
    opacity: 0.85;
  }
  /* A narrow pane keeps the three that tell the story: in / thinking / out. */
  @container (max-width: 420px) {
    .u-part.u-cache_read,
    .u-part.u-cache_write,
    .u-model {
      display: none;
    }
  }
</style>
