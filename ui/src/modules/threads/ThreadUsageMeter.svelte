<script lang="ts">
  // The footer's Usage button as a meter for the focused thread's agent: its
  // tightest plan window ("5h 62%" used), warning at ≥ 80 %, danger at ≥ 95 %,
  // every window and its reset in the tooltip; click opens Usage. No fresh
  // numbers for that agent: the plain icon. Lazy (threadLazy.svelte.ts): the
  // Usage wording stays out of the Threads page chunk.
  import Icon from '../../lib/components/Icon.svelte';
  import { limits } from '../../lib/stores/usageLimits.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { providerMeter } from '../usage/limitsText'; // ui-guards: allow — the Usage page's own wording for the same numbers
  import { runtimeThreads } from './runtime/runtimeThreads.svelte';
  import { UsageView } from './threadLazy.svelte';
  import { threadsChrome } from './threadsChrome.svelte';
  import { threads } from './threadsStore.svelte';

  // The Agents inbox passes its focused session's provider and opens the Usage page.
  let { provider: forProvider, onopen }: { provider?: string | null; onopen?: () => void } = $props();
  $effect(() => limits.mount());
  const provider = $derived.by(() => {
    if (forProvider !== undefined) return forProvider;
    const id = threads.focusedSessionId;
    if (!id || threads.newDraftMode) return null;
    return runtimeThreads.get(id)?.provider ?? ws.getSession(id)?.provider ?? null;
  });
  const meter = $derived(provider && limits.data ? providerMeter(limits.data.accounts, provider, limits.data.stale_after_secs, limits.now) : null);
</script>

{#if meter}
  <button
    type="button"
    class="th-meter {meter.tone}"
    aria-label="Usage: {meter.short} window {meter.used}% used. Open Usage"
    title={[...meter.lines, 'Open Usage'].join('\n')}
    data-testid="threads-open-usage"
    onpointerenter={() => UsageView.prefetch()}
    onclick={() => (onopen ? onopen() : (threadsChrome.sheet = 'usage'))}
  >
    <span>{meter.short}</span>
    <span class="th-meter-bar" aria-hidden="true"><span class="th-meter-fill" style:inline-size="{Math.min(100, meter.used)}%"></span></span>
    <span class="th-meter-pct" data-testid="threads-usage-meter">{meter.used}%</span>
  </button>
{:else}
  <button
    type="button"
    class="icon-btn"
    aria-label="Usage statistics"
    title="Usage statistics"
    data-testid="threads-open-usage"
    onpointerenter={() => UsageView.prefetch()}
    onclick={() => (onopen ? onopen() : (threadsChrome.sheet = 'usage'))}
  >
    <Icon name="chart" size={14} />
  </button>
{/if}

<style>
  .th-meter {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-3);
    min-block-size: 28px;
    padding-inline: var(--sp-3);
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .th-meter:hover {
    background: var(--hover);
    color: var(--text);
  }
  .th-meter-bar {
    inline-size: 36px;
    block-size: 4px;
    border-radius: var(--radius-s);
    background: var(--surface-3);
    overflow: hidden;
  }
  .th-meter-fill {
    display: block;
    block-size: 100%;
    background: var(--text-dim);
  }
  .th-meter.warning {
    color: var(--warning);
  }
  .th-meter.warning .th-meter-fill {
    background: var(--warning);
  }
  .th-meter.danger {
    color: var(--danger);
  }
  .th-meter.danger .th-meter-fill {
    background: var(--danger);
  }
  @media (hover: none), (pointer: coarse) {
    .th-meter {
      min-block-size: var(--hit-min);
    }
  }
</style>
