<script lang="ts">
  // Top-of-page ingestion health for the Usage page: the transcript tailer
  // retrying / stopped (PC-2), the usage database down, the journal filling.
  // The totals below go stale while any of these holds; say so, with why.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import type { UsageStatus } from '../../lib/api/usage.svelte';
  import { usageHealthLines } from './usageHealth';

  let { status }: { status: UsageStatus | null | undefined } = $props();
  const lines = $derived(usageHealthLines(status));
</script>

{#each lines as line (line.key)}
  <div class="health-banner" data-testid="usage-health-{line.key}" use:announceShown={{ text: line.text, assertive: true }}>
    <Icon name="warning" size={14} />
    <span>{line.text}</span>
  </div>
{/each}

<style>
  .health-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block: 6px;
    padding-inline: 12px 8px;
    margin: 0 0 14px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
    color: var(--text);
    font-size: var(--fs-m);
  }
  .health-banner > :global(svg:first-child) {
    color: var(--warning);
    flex-shrink: 0;
  }
  .health-banner span {
    flex: 1;
    min-width: 0;
  }
</style>
