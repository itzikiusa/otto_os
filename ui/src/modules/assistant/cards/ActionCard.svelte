<script lang="ts">
  // The frame every in-thread action card shares (reminder, task, browser,
  // approval): an icon + bold kind + dim one-line summary header with a state
  // pill at the trailing edge, a body, and an optional footer well for its
  // actions. Hairline border, no shadow (foundations §5).
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';

  interface Props {
    icon: IconName;
    kind: string;
    summary?: string;
    /** Attention ring for a card that is waiting on you. */
    attention?: boolean;
    pill?: Snippet;
    children?: Snippet;
    footer?: Snippet;
    testid?: string;
  }
  let { icon, kind, summary, attention = false, pill, children, footer, testid }: Props = $props();
</script>

<section class="card" class:attention data-testid={testid} aria-label={summary ? `${kind}: ${summary}` : kind}>
  <header class="head">
    <span class="ic" aria-hidden="true"><Icon name={icon} size={14} /></span>
    <strong class="kind">{kind}</strong>
    {#if summary}<span class="summary" title={summary}>{summary}</span>{/if}
    {#if pill}<span class="pill-slot">{@render pill()}</span>{/if}
  </header>
  {#if children}<div class="body">{@render children()}</div>{/if}
  {#if footer}<footer class="foot">{@render footer()}</footer>{/if}
</section>

<style>
  .card {
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    overflow: hidden;
    min-width: 0;
  }
  .card.attention {
    border-color: color-mix(in srgb, var(--warning) 45%, var(--border));
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    padding: 6px 12px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-s);
    min-width: 0;
  }
  .ic {
    display: inline-flex;
    color: var(--text-dim);
  }
  .card.attention .ic {
    color: var(--warning);
  }
  .kind {
    font-size: var(--fs-m);
    font-weight: 600;
    white-space: nowrap;
  }
  .summary {
    color: var(--text-dim);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pill-slot {
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .body {
    padding: 10px 12px;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .foot {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 12px;
    border-top: 1px solid var(--border);
    background: var(--surface-2);
  }
</style>
